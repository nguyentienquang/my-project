package com.watchcam.android.service

import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import com.watchcam.android.MainActivity
import com.watchcam.android.R
import com.watchcam.android.WatchCamApp
import com.watchcam.android.audio.AudioCaptureManager
import com.watchcam.android.audio.AudioPlaybackManager
import com.watchcam.android.camera.CameraManager
import com.watchcam.android.email.SmtpClient
import com.watchcam.android.nativebridge.WatchCamNative
import com.watchcam.android.network.NetworkHelper
import com.watchcam.android.status.DeviceTelemetry
import com.watchcam.android.status.DeviceTelemetryManager
import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit

class SurveillanceService : LifecycleService() {

    companion object {
        private const val TAG = "WatchCam_Service"
        const val NOTIFICATION_ID = 1001

        const val ACTION_START = "com.watchcam.android.action.START"
        const val ACTION_STOP = "com.watchcam.android.action.STOP"
        const val ACTION_TRIGGER_RECORDING = "com.watchcam.android.action.TRIGGER_RECORDING"
        const val EXTRA_PORT = "extra_port"

        var isRunning: Boolean = false
            private set

        var activeCameraManager: CameraManager? = null
            private set

        var onStatusListener: ((Boolean) -> Unit)? = null

        var serviceStartTimeMillis: Long = 0L
            private set

        @Volatile
        var latestTelemetry: DeviceTelemetry? = null
            private set
    }

    private var wakeLock: PowerManager.WakeLock? = null
    private var cameraManager: CameraManager? = null
    private var audioCaptureManager: AudioCaptureManager? = null
    private var audioPlaybackManager: AudioPlaybackManager? = null
    private val emailExecutor = Executors.newSingleThreadExecutor()
    private val telemetryExecutor = Executors.newSingleThreadScheduledExecutor()
    private var telemetryFuture: ScheduledFuture<*>? = null

    override fun onCreate() {
        super.onCreate()
        val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = powerManager.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "WatchCam::SurveillanceWakeLock")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)

        when (intent?.action) {
            ACTION_START -> {
                val port = intent.getIntExtra(EXTRA_PORT, 8080)
                startSurveillance(port)
            }
            ACTION_STOP -> {
                stopSurveillance()
            }
            ACTION_TRIGGER_RECORDING -> {
                val cam = cameraManager ?: activeCameraManager
                if (cam != null) {
                    val eventId = intent.getStringExtra("eventId") ?: "evt_${System.currentTimeMillis()}"
                    val res = cam.getCurrentResolution()
                    val rot = cam.getLastRotation()
                    Log.i(TAG, "Triggering video recording for eventId: $eventId (${res.width}x${res.height}, rot=$rot)")
                    handleMotionAlert(cam, 0.88f, res.width, res.height, rot, eventId)
                } else {
                    Log.w(TAG, "Cannot trigger recording: cameraManager is null")
                }
            }
        }

        return START_NOT_STICKY
    }

    private fun startSurveillance(port: Int) {
        if (isRunning) return
        Log.i(TAG, "Starting Surveillance Service on port $port")

        // 1. Acquire WakeLock
        wakeLock?.acquire(24 * 60 * 60 * 1000L /* 24 hours */)

        // 2. Initialize Native Surveillance Engine & Server
        val storageDir = File(filesDir, "snapshots").apply { mkdirs() }.absolutePath
        val dbPath = File(filesDir, "watchcam.db").absolutePath

        val serverInitOk = WatchCamNative.initServer(storageDir, dbPath, port)
        if (!serverInitOk) {
            Log.e(TAG, "Failed to initialize native server")
        }

        // Load persisted email settings and sync to native engine
        try {
            val prefs = getSharedPreferences("watchcam_prefs", Context.MODE_PRIVATE)
            val emailEnabled = prefs.getBoolean("email_alerts_enabled", false)
            val host = prefs.getString("smtp_host", "smtp.gmail.com") ?: "smtp.gmail.com"
            val smtpPort = prefs.getInt("smtp_port", 587)
            val username = prefs.getString("smtp_username", "") ?: ""
            val password = prefs.getString("smtp_password", "") ?: ""
            val recipient = prefs.getString("smtp_recipient", "") ?: ""

            WatchCamNative.updateEmailConfig(emailEnabled, host, smtpPort, username, password, recipient)
            Log.i(TAG, "Synced email config from prefs to native engine (enabled=$emailEnabled, host=$host, port=$smtpPort)")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to sync email config from prefs", e)
        }

        // 3. Start Foreground Notification
        val notification = createNotification(port)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            )
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }

        // 4. Initialize Two-Way Audio Playback
        try {
            val playback = AudioPlaybackManager(this)
            playback.start()
            this.audioPlaybackManager = playback
            WatchCamNative.audioPlaybackManager = playback
            Log.i(TAG, "Two-way audio playback initialized")
        } catch (e: Exception) {
            Log.w(TAG, "Failed to initialize audio playback manager", e)
        }

        // 5. Initialize and start unified CameraX capture pipeline
        val cam = CameraManager(this, this, null).apply {
            onMonitoringStateChanged = { active ->
                Log.i(TAG, "Monitoring state changed: isMonitoring=$active")
            }
            onMotionAlertWithFrame = { score, width, height, rotation ->
                handleMotionAlert(this, score, width, height, rotation)
            }
            onTestEmailRequested = {
                handleTestEmail()
            }
            startCamera()
        }
        cameraManager = cam
        activeCameraManager = cam

        // 6. Initialize and start Audio Capture (Mic -> WebSocket & Motion Video Clips)
        try {
            val capture = AudioCaptureManager(this)
            val captureStarted = capture.startCapture { buffer, bytesRead, _ ->
                val recorder = cameraManager?.videoRecorder
                if (recorder != null && recorder.isRecording()) {
                    recorder.onAudioFrameAvailable(buffer, bytesRead)
                }
            }
            this.audioCaptureManager = capture
            Log.i(TAG, "Audio capture started: success=$captureStarted")
        } catch (e: Exception) {
            Log.w(TAG, "Failed to start AudioCaptureManager", e)
        }

        serviceStartTimeMillis = System.currentTimeMillis()
        isRunning = true
        onStatusListener?.invoke(true)

        telemetryFuture?.cancel(false)
        telemetryFuture = telemetryExecutor.scheduleWithFixedDelay({
            if (!isRunning) return@scheduleWithFixedDelay
            try {
                val telemetry = DeviceTelemetryManager.gatherTelemetry(
                    this@SurveillanceService,
                    serviceStartTimeMillis
                )
                latestTelemetry = telemetry
                val jsonStr = DeviceTelemetryManager.toJsonString(telemetry)
                WatchCamNative.updateDeviceStatus(jsonStr)

                // Synchronize any external email config updates from native engine into SharedPreferences
                try {
                    val statusJson = WatchCamNative.getStatusJson()
                    if (statusJson.isNotEmpty() && statusJson != "{}") {
                        val statusObj = JSONObject(statusJson)
                        val prefs = getSharedPreferences("watchcam_prefs", Context.MODE_PRIVATE)
                        val rEnabled = statusObj.optBoolean("emailAlertsEnabled", prefs.getBoolean("email_alerts_enabled", false))
                        val rHost = statusObj.optString("smtpHost", "")
                        val rPort = statusObj.optInt("smtpPort", 0)
                        val rUser = statusObj.optString("smtpUsername", "")
                        val rPass = statusObj.optString("smtpPassword", "")
                        val rRcpt = statusObj.optString("smtpRecipient", "")

                        val pEnabled = prefs.getBoolean("email_alerts_enabled", false)
                        val pHost = prefs.getString("smtp_host", "smtp.gmail.com") ?: "smtp.gmail.com"
                        val pPort = prefs.getInt("smtp_port", 587)
                        val pUser = prefs.getString("smtp_username", "") ?: ""
                        val pPass = prefs.getString("smtp_password", "") ?: ""
                        val pRcpt = prefs.getString("smtp_recipient", "") ?: ""

                        if (rEnabled != pEnabled ||
                            (rHost.isNotEmpty() && rHost != pHost) ||
                            (rPort > 0 && rPort != pPort) ||
                            (rUser.isNotEmpty() && rUser != pUser) ||
                            (rPass.isNotEmpty() && rPass != pPass) ||
                            (rRcpt.isNotEmpty() && rRcpt != pRcpt)) {
                            prefs.edit().apply {
                                putBoolean("email_alerts_enabled", rEnabled)
                                if (rHost.isNotEmpty()) putString("smtp_host", rHost)
                                if (rPort > 0) putInt("smtp_port", rPort)
                                if (rUser.isNotEmpty()) putString("smtp_username", rUser)
                                if (rPass.isNotEmpty()) putString("smtp_password", rPass)
                                if (rRcpt.isNotEmpty()) putString("smtp_recipient", rRcpt)
                                apply()
                            }
                            Log.i(TAG, "Persisted updated email configuration from native engine to watchcam_prefs")
                        }
                    }
                } catch (e: Exception) {
                    Log.w(TAG, "Error syncing email config to prefs: ${e.message}")
                }
            } catch (e: Exception) {
                Log.w(TAG, "Error updating device telemetry: ${e.message}")
            }
        }, 0L, 2L, TimeUnit.SECONDS)
    }

    private fun stopSurveillance() {
        if (!isRunning && cameraManager == null) return
        Log.i(TAG, "Stopping Surveillance Service")

        telemetryFuture?.cancel(false)
        telemetryFuture = null
        serviceStartTimeMillis = 0L

        cameraManager?.release()
        cameraManager = null
        activeCameraManager = null

        audioCaptureManager?.release()
        audioCaptureManager = null

        WatchCamNative.audioPlaybackManager = null
        audioPlaybackManager?.release()
        audioPlaybackManager = null

        WatchCamNative.stopServer()

        if (wakeLock?.isHeld == true) {
            wakeLock?.release()
        }

        isRunning = false
        onStatusListener?.invoke(false)

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
        stopSelf()
    }

    private fun createNotification(port: Int): Notification {
        val ip = NetworkHelper.getLocalIpAddress()
        val url = "http://$ip:$port"

        val openAppIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val pendingOpenIntent = PendingIntent.getActivity(
            this,
            0,
            openAppIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val stopIntent = Intent(this, SurveillanceService::class.java).apply {
            action = ACTION_STOP
        }
        val pendingStopIntent = PendingIntent.getService(
            this,
            1,
            stopIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, WatchCamApp.NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setContentTitle(getString(R.string.notification_title))
            .setContentText(getString(R.string.notification_text, url))
            .setContentIntent(pendingOpenIntent)
            .addAction(
                android.R.drawable.ic_menu_close_clear_cancel,
                getString(R.string.action_stop_surveillance),
                pendingStopIntent
            )
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }

    private fun handleMotionAlert(
        cam: CameraManager,
        score: Float,
        width: Int,
        height: Int,
        rotation: Int,
        customEventId: String? = null
    ) {
        val latestJson = WatchCamNative.getLatestEventJson()
        var eventId = customEventId ?: "evt_${System.currentTimeMillis()}"
        var snapshotPath: String? = null
        var entityType = "motion"
        var confidence = score
        if (customEventId == null && latestJson.isNotEmpty() && latestJson != "{}") {
            try {
                val obj = JSONObject(latestJson)
                eventId = obj.optString("id", eventId)
                snapshotPath = obj.optString("snapshot_path").takeIf { it.isNotEmpty() }
                entityType = obj.optString("entity_type", "motion")
                confidence = obj.optDouble("confidence", score.toDouble()).toFloat()
            } catch (e: Exception) {
                Log.w(TAG, "Error parsing latest event JSON", e)
            }
        }

        val storageDir = File(filesDir, "snapshots").apply { mkdirs() }
        val videoFile = File(storageDir, "motion_${eventId}.mp4")

        val started = cam.videoRecorder.startRecording(
            width = width,
            height = height,
            rotation = rotation,
            outputFile = videoFile,
            durationMs = 4000L
        ) { recordedFile ->
            if (recordedFile != null) {
                Log.i(TAG, "Video recording finished: ${recordedFile.absolutePath}, notifying native engine")
                WatchCamNative.notifyVideoRecorded(eventId, recordedFile.absolutePath)
            } else {
                Log.w(TAG, "Video recording returned null or invalid file for event $eventId")
            }
            sendEmailAlertAsync(eventId, score, snapshotPath, recordedFile, entityType, confidence)
        }

        if (!started) {
            sendEmailAlertAsync(eventId, score, snapshotPath, null, entityType, confidence)
        }
    }

    private fun sendEmailAlertAsync(
        eventId: String,
        score: Float,
        snapshotPath: String?,
        videoFile: File?,
        entityType: String = "motion",
        confidence: Float = score
    ) {
        emailExecutor.execute {
            try {
                val statusJson = WatchCamNative.getStatusJson()
                if (statusJson.isEmpty() || statusJson == "{}") return@execute
                val json = JSONObject(statusJson)

                val emailEnabled = json.optBoolean("emailAlertsEnabled", false)
                if (!emailEnabled) {
                    Log.d(TAG, "Email alerts disabled in config, skipping email dispatch")
                    return@execute
                }

                val host = json.optString("smtpHost", "smtp.gmail.com")
                val port = json.optInt("smtpPort", 587)
                val username = json.optString("smtpUsername", "")
                val password = json.optString("smtpPassword", "")
                val recipient = json.optString("smtpRecipient", "")

                if (username.isBlank() || password.isBlank() || recipient.isBlank()) {
                    Log.w(TAG, "SMTP credentials or recipient not configured")
                    return@execute
                }

                val config = SmtpClient.SmtpConfig(
                    host = host,
                    port = port,
                    username = username,
                    password = password,
                    recipient = recipient,
                    enabled = true
                )

                val attachments = mutableListOf<File>()
                if (snapshotPath != null) {
                    val sFile = File(snapshotPath)
                    if (sFile.exists() && sFile.length() > 0) {
                        attachments.add(sFile)
                    }
                }
                if (videoFile != null && videoFile.exists() && videoFile.length() > 0) {
                    attachments.add(videoFile)
                }

                val (entityTitleVi, entityDescVi) = when (entityType.lowercase(Locale.ROOT)) {
                    "person" -> Pair("Phát hiện người xâm nhập", "Người")
                    "object" -> Pair("Phát hiện vật thể", "Vật thể")
                    else -> Pair("Phát hiện chuyển động", "Chuyển động chung")
                }

                val confPct = (confidence * 100).toInt()
                val scorePct = (score * 100).toInt()
                val timeStr = SimpleDateFormat("HH:mm:ss dd/MM/yyyy", Locale.getDefault()).format(Date())
                val subject = "[WatchCam Cảnh Báo] $entityTitleVi (${confPct}%) lúc $timeStr"
                val bodyHtml = """
                    <h2>Cảnh Báo $entityTitleVi</h2>
                    <p>Hệ thống giám sát <b>WatchCam</b> đã nhận diện sự kiện an ninh.</p>
                    <ul>
                        <li><b>Mã sự kiện:</b> $eventId</li>
                        <li><b>Thời gian:</b> $timeStr</li>
                        <li><b>Thực thể phát hiện:</b> $entityDescVi</li>
                        <li><b>Độ tin cậy nhận diện:</b> $confPct%</li>
                        <li><b>Mức độ chuyển động:</b> $scorePct%</li>
                        <li><b>Thiết bị:</b> IvyPad Smart 5</li>
                    </ul>
                    <p>Ảnh chụp và đoạn video clip ghi lại sự kiện được đính kèm trong email này.</p>
                    <hr/>
                    <small>Email được gửi tự động từ hệ thống giám sát an ninh WatchCam.</small>
                """.trimIndent()

                val result = SmtpClient.sendEmail(config, subject, bodyHtml, attachments)
                if (result.isSuccess) {
                    Log.i(TAG, "Motion email alert sent successfully to $recipient (attachments: ${attachments.size})")
                } else {
                    Log.e(TAG, "Failed to send motion email alert", result.exceptionOrNull())
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error during SMTP alert dispatch", e)
            }
        }
    }

    private fun handleTestEmail() {
        emailExecutor.execute {
            try {
                val statusJson = WatchCamNative.getStatusJson()
                if (statusJson.isEmpty() || statusJson == "{}") return@execute
                val json = JSONObject(statusJson)

                val host = json.optString("smtpHost", "smtp.gmail.com")
                val port = json.optInt("smtpPort", 587)
                val username = json.optString("smtpUsername", "")
                val password = json.optString("smtpPassword", "")
                val recipient = json.optString("smtpRecipient", "")

                if (username.isBlank() || password.isBlank() || recipient.isBlank()) {
                    Log.w(TAG, "SMTP credentials or recipient not configured for test email")
                    return@execute
                }

                val config = SmtpClient.SmtpConfig(
                    host = host,
                    port = port,
                    username = username,
                    password = password,
                    recipient = recipient,
                    enabled = true
                )

                val timeStr = SimpleDateFormat("HH:mm:ss dd/MM/yyyy", Locale.getDefault()).format(Date())
                val subject = "[WatchCam] Thử Nghiệm Kết Nối Email SMTP Thành Công"
                val bodyHtml = """
                    <h2>Kết Nối Gmail SMTP Thành Công</h2>
                    <p>Hệ thống camera giám sát an ninh <b>WatchCam</b> đã kết nối thành công tới máy chủ SMTP Gmail.</p>
                    <p>Cấu hình gửi email cảnh báo của bạn đã hoạt động chính xác.</p>
                    <p><b>Thời gian thử nghiệm:</b> $timeStr</p>
                    <hr/>
                    <small>WatchCam Security System</small>
                """.trimIndent()

                val result = SmtpClient.sendEmail(config, subject, bodyHtml, emptyList())
                if (result.isSuccess) {
                    Log.i(TAG, "Test email sent successfully to $recipient")
                } else {
                    Log.e(TAG, "Failed to send test email", result.exceptionOrNull())
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error sending test email", e)
            }
        }
    }

    override fun onDestroy() {
        stopSurveillance()
        telemetryExecutor.shutdownNow()
        emailExecutor.shutdown()
        super.onDestroy()
    }
}
