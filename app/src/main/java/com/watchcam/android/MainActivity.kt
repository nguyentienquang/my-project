package com.watchcam.android

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.method.PasswordTransformationMethod
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.SeekBar
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.widget.SwitchCompat
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import com.watchcam.android.email.SmtpClient
import com.watchcam.android.nativebridge.WatchCamNative
import com.watchcam.android.network.NetworkHelper
import com.watchcam.android.service.SurveillanceService
import com.watchcam.android.status.DeviceTelemetryManager
import org.json.JSONObject
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.Executors

class MainActivity : AppCompatActivity() {

    private lateinit var cameraPreviewView: PreviewView
    private lateinit var chipStatus: TextView
    private lateinit var layoutPermission: LinearLayout
    private lateinit var btnGrantPermission: Button
    private lateinit var textCameraName: TextView
    private lateinit var textPreviewFps: TextView
    private lateinit var btnSwitchCamera: Button
    private lateinit var textServerUrl: TextView
    private lateinit var textPairingCode: TextView
    private lateinit var btnRefreshPairingCode: Button
    private lateinit var btnRevokeSessions: Button
    private lateinit var btnToggleSurveillance: Button
    private lateinit var switchMotionDetection: SwitchCompat
    private lateinit var textSensitivityLabel: TextView
    private lateinit var seekSensitivity: SeekBar
    private lateinit var textViewersCount: TextView
    private lateinit var textTotalEvents: TextView

    // Tablet Hardware Health Views
    private lateinit var textHealthSummary: TextView
    private lateinit var chipBattery: TextView
    private lateinit var textBatterySub: TextView
    private lateinit var chipThermal: TextView
    private lateinit var textThermalSub: TextView
    private lateinit var chipRam: TextView
    private lateinit var textRamSub: TextView
    private lateinit var chipStorage: TextView
    private lateinit var textStorageSub: TextView
    private lateinit var chipWifi: TextView
    private lateinit var chipUptime: TextView

    // Email Alert Views
    private lateinit var switchEmailAlerts: SwitchCompat
    private lateinit var editSmtpUsername: EditText
    private lateinit var editSmtpPassword: EditText
    private lateinit var btnTogglePasswordVisibility: Button
    private lateinit var editSmtpRecipient: EditText
    private lateinit var editSmtpHost: EditText
    private lateinit var editSmtpPort: EditText
    private lateinit var textEmailStatus: TextView
    private lateinit var btnSaveEmailConfig: Button
    private lateinit var btnTestEmail: Button
    private val emailExecutor = Executors.newSingleThreadExecutor()

    private val uiHandler = Handler(Looper.getMainLooper())
    private val serverPort = 8080

    private val statusUpdateRunnable = object : Runnable {
        override fun run() {
            updateStatusFromNative()
            updateDeviceHealthDisplay()
            updateNetworkUrlDisplay()
            uiHandler.postDelayed(this, 1000)
        }
    }

    private val permissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { permissions ->
        val cameraGranted = permissions[Manifest.permission.CAMERA] ?: false
        if (cameraGranted) {
            layoutPermission.visibility = View.GONE
            if (!SurveillanceService.isRunning) {
                startSurveillanceService()
            }
        } else {
            layoutPermission.visibility = View.VISIBLE
            Toast.makeText(
                this,
                getString(R.string.camera_permission_required),
                Toast.LENGTH_LONG
            ).show()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        initViews()
        loadSavedEmailConfig()
        setupListeners()
        checkAndRequestPermissions()
        handleTriggerRecording(intent)
    }

    override fun onNewIntent(intent: Intent?) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleTriggerRecording(intent)
    }

    private fun handleTriggerRecording(intent: Intent?) {
        if (intent?.action == SurveillanceService.ACTION_TRIGGER_RECORDING) {
            val eventId = intent.getStringExtra("eventId") ?: "evt_${System.currentTimeMillis()}"
            val serviceIntent = Intent(this, SurveillanceService::class.java).apply {
                action = SurveillanceService.ACTION_TRIGGER_RECORDING
                putExtra("eventId", eventId)
            }
            startService(serviceIntent)
        }
    }

    private fun initViews() {
        cameraPreviewView = findViewById(R.id.cameraPreviewView)
        chipStatus = findViewById(R.id.chipStatus)
        layoutPermission = findViewById(R.id.layoutPermission)
        btnGrantPermission = findViewById(R.id.btnGrantPermission)
        textCameraName = findViewById(R.id.textCameraName)
        textPreviewFps = findViewById(R.id.textPreviewFps)
        btnSwitchCamera = findViewById(R.id.btnSwitchCamera)
        textServerUrl = findViewById(R.id.textServerUrl)
        textPairingCode = findViewById(R.id.textPairingCode)
        btnRefreshPairingCode = findViewById(R.id.btnRefreshPairingCode)
        btnRevokeSessions = findViewById(R.id.btnRevokeSessions)
        btnToggleSurveillance = findViewById(R.id.btnToggleSurveillance)
        switchMotionDetection = findViewById(R.id.switchMotionDetection)
        textSensitivityLabel = findViewById(R.id.textSensitivityLabel)
        seekSensitivity = findViewById(R.id.seekSensitivity)
        textViewersCount = findViewById(R.id.textViewersCount)
        textTotalEvents = findViewById(R.id.textTotalEvents)

        textHealthSummary = findViewById(R.id.textHealthSummary)
        chipBattery = findViewById(R.id.chipBattery)
        textBatterySub = findViewById(R.id.textBatterySub)
        chipThermal = findViewById(R.id.chipThermal)
        textThermalSub = findViewById(R.id.textThermalSub)
        chipRam = findViewById(R.id.chipRam)
        textRamSub = findViewById(R.id.textRamSub)
        chipStorage = findViewById(R.id.chipStorage)
        textStorageSub = findViewById(R.id.textStorageSub)
        chipWifi = findViewById(R.id.chipWifi)
        chipUptime = findViewById(R.id.chipUptime)

        // Email Alert Views
        switchEmailAlerts = findViewById(R.id.switchEmailAlerts)
        editSmtpUsername = findViewById(R.id.editSmtpUsername)
        editSmtpPassword = findViewById(R.id.editSmtpPassword)
        btnTogglePasswordVisibility = findViewById(R.id.btnTogglePasswordVisibility)
        editSmtpRecipient = findViewById(R.id.editSmtpRecipient)
        editSmtpHost = findViewById(R.id.editSmtpHost)
        editSmtpPort = findViewById(R.id.editSmtpPort)
        textEmailStatus = findViewById(R.id.textEmailStatus)
        btnSaveEmailConfig = findViewById(R.id.btnSaveEmailConfig)
        btnTestEmail = findViewById(R.id.btnTestEmail)

        updateNetworkUrlDisplay()
    }

    private fun loadSavedEmailConfig() {
        val prefs = getSharedPreferences("watchcam_prefs", Context.MODE_PRIVATE)
        // One-time security migration to ensure any legacy credentials on development devices are sanitized
        if (!prefs.getBoolean("privacy_sanitized_v1", false)) {
            prefs.edit()
                .putString("smtp_username", "")
                .putString("smtp_password", "")
                .putString("smtp_recipient", "")
                .putBoolean("privacy_sanitized_v1", true)
                .apply()
        }

        var emailEnabled = prefs.getBoolean("email_alerts_enabled", false)
        var host = prefs.getString("smtp_host", "smtp.gmail.com") ?: "smtp.gmail.com"
        var port = prefs.getInt("smtp_port", 587)
        var username = prefs.getString("smtp_username", "") ?: ""
        var password = prefs.getString("smtp_password", "") ?: ""
        var recipient = prefs.getString("smtp_recipient", "") ?: ""

        // If credentials are empty in prefs, attempt to retrieve from native engine status
        if (username.isEmpty()) {
            try {
                val statusJson = WatchCamNative.getStatusJson()
                if (statusJson.isNotEmpty() && statusJson != "{}") {
                    val json = JSONObject(statusJson)
                    val rUser = json.optString("smtpUsername", "")
                    val rRecipient = json.optString("smtpRecipient", "")
                    if (rUser.isNotEmpty()) {
                        emailEnabled = json.optBoolean("emailAlertsEnabled", emailEnabled)
                        host = json.optString("smtpHost", host)
                        port = json.optInt("smtpPort", port)
                        username = rUser
                        password = json.optString("smtpPassword", password)
                        recipient = rRecipient

                        prefs.edit()
                            .putBoolean("email_alerts_enabled", emailEnabled)
                            .putString("smtp_host", host)
                            .putInt("smtp_port", port)
                            .putString("smtp_username", username)
                            .putString("smtp_password", password)
                            .putString("smtp_recipient", recipient)
                            .apply()
                    }
                }
            } catch (_: Exception) {}
        }

        switchEmailAlerts.isChecked = emailEnabled
        editSmtpHost.setText(host)
        editSmtpPort.setText(port.toString())
        editSmtpUsername.setText(username)
        editSmtpPassword.setText(password)
        editSmtpRecipient.setText(recipient)

        // Sync to native surveillance engine
        WatchCamNative.updateEmailConfig(emailEnabled, host, port, username, password, recipient)
    }

    private fun showEmailStatus(message: String, isError: Boolean, isPending: Boolean = false, isWarning: Boolean = false) {
        textEmailStatus.visibility = View.VISIBLE
        textEmailStatus.text = message
        val color = when {
            isPending -> ContextCompat.getColor(this, R.color.accent_blue)
            isError -> ContextCompat.getColor(this, R.color.accent_red)
            isWarning -> ContextCompat.getColor(this, R.color.accent_amber)
            else -> ContextCompat.getColor(this, R.color.accent_green)
        }
        textEmailStatus.setTextColor(color)
    }

    private fun onEmailSwitchChanged(isChecked: Boolean) {
        val host = editSmtpHost.text.toString().trim().ifEmpty { "smtp.gmail.com" }
        val port = editSmtpPort.text.toString().trim().toIntOrNull() ?: 587
        val username = editSmtpUsername.text.toString().trim()
        val password = editSmtpPassword.text.toString().trim()
        val recipient = editSmtpRecipient.text.toString().trim()

        getSharedPreferences("watchcam_prefs", Context.MODE_PRIVATE).edit()
            .putBoolean("email_alerts_enabled", isChecked)
            .apply()

        WatchCamNative.updateEmailConfig(isChecked, host, port, username, password, recipient)
        if (isChecked) {
            if (username.isBlank() || password.isBlank() || recipient.isBlank()) {
                showEmailStatus("Lưu ý: Đã bật cảnh báo, nhưng chưa điền đủ thông tin Gmail. Vui lòng nhập Gmail, Mật khẩu ứng dụng và bấm Lưu!", isError = false, isWarning = true)
            } else {
                showEmailStatus("Đã bật cảnh báo qua Gmail", isError = false)
            }
        } else {
            showEmailStatus("Đã tắt cảnh báo qua Gmail", isError = false)
        }
    }

    private fun setupListeners() {
        btnGrantPermission.setOnClickListener {
            checkAndRequestPermissions()
        }

        btnToggleSurveillance.setOnClickListener {
            if (SurveillanceService.isRunning) {
                stopSurveillanceService()
            } else {
                startSurveillanceService()
            }
        }

        btnSwitchCamera.setOnClickListener {
            SurveillanceService.activeCameraManager?.switchCamera()
        }

        btnRefreshPairingCode.setOnClickListener {
            val newCode = WatchCamNative.regeneratePairingCode()
            textPairingCode.text = formatPairingCode(newCode)
            Toast.makeText(this, "Đã tạo mã ghép nối mới: $newCode", Toast.LENGTH_SHORT).show()
        }

        btnRevokeSessions.setOnClickListener {
            val dbPath = java.io.File(filesDir, "watchcam.db").absolutePath
            WatchCamNative.revokeAllSessionsWithDb(dbPath)
            Toast.makeText(this, "Đã thu hồi tất cả phiên kết nối", Toast.LENGTH_SHORT).show()
        }

        seekSensitivity.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(seekBar: SeekBar?, progress: Int, fromUser: Boolean) {
                textSensitivityLabel.text = getString(R.string.label_sensitivity, progress)
            }
            override fun onStartTrackingTouch(seekBar: SeekBar?) {}
            override fun onStopTrackingTouch(seekBar: SeekBar?) {
                val progress = seekBar?.progress ?: 70
                WatchCamNative.updateConfig(
                    switchMotionDetection.isChecked,
                    progress,
                    0
                )
            }
        })

        switchMotionDetection.setOnCheckedChangeListener { _, isChecked ->
            WatchCamNative.updateConfig(
                isChecked,
                seekSensitivity.progress,
                0
            )
        }

        // Email Password Visibility Toggle
        btnTogglePasswordVisibility.setOnClickListener {
            if (editSmtpPassword.transformationMethod is PasswordTransformationMethod) {
                editSmtpPassword.transformationMethod = null
                btnTogglePasswordVisibility.text = "Ẩn"
            } else {
                editSmtpPassword.transformationMethod = PasswordTransformationMethod.getInstance()
                btnTogglePasswordVisibility.text = "Hiện"
            }
            editSmtpPassword.setSelection(editSmtpPassword.text.length)
        }

        // Email Alert Switch Listener
        switchEmailAlerts.setOnCheckedChangeListener { _, isChecked ->
            onEmailSwitchChanged(isChecked)
        }

        // Save Email Config Button
        btnSaveEmailConfig.setOnClickListener {
            val host = editSmtpHost.text.toString().trim().ifEmpty { "smtp.gmail.com" }
            val portStr = editSmtpPort.text.toString().trim()
            val port = portStr.toIntOrNull() ?: 587
            val username = editSmtpUsername.text.toString().trim()
            val password = editSmtpPassword.text.toString().trim()
            val recipient = editSmtpRecipient.text.toString().trim()
            val enabled = switchEmailAlerts.isChecked

            if (port < 1 || port > 65535) {
                showEmailStatus("Cổng SMTP không hợp lệ (1 - 65535)", isError = true)
                return@setOnClickListener
            }

            if (enabled) {
                if (username.isEmpty() || !username.contains("@")) {
                    showEmailStatus("Vui lòng nhập địa chỉ Gmail gửi cảnh báo hợp lệ", isError = true)
                    return@setOnClickListener
                }
                if (password.isEmpty()) {
                    showEmailStatus("Vui lòng nhập mật khẩu ứng dụng Google (16 ký tự)", isError = true)
                    return@setOnClickListener
                }
                if (recipient.isEmpty() || !recipient.contains("@")) {
                    showEmailStatus("Vui lòng nhập địa chỉ Gmail người nhận hợp lệ", isError = true)
                    return@setOnClickListener
                }
            }

            getSharedPreferences("watchcam_prefs", Context.MODE_PRIVATE).edit()
                .putBoolean("email_alerts_enabled", enabled)
                .putString("smtp_host", host)
                .putInt("smtp_port", port)
                .putString("smtp_username", username)
                .putString("smtp_password", password)
                .putString("smtp_recipient", recipient)
                .apply()

            WatchCamNative.updateEmailConfig(enabled, host, port, username, password, recipient)
            showEmailStatus("Đã lưu cấu hình Email thành công!", isError = false)
            Toast.makeText(this, "Đã lưu cấu hình Email thành công!", Toast.LENGTH_SHORT).show()
        }

        // Test Email Button
        btnTestEmail.setOnClickListener {
            val host = editSmtpHost.text.toString().trim().ifEmpty { "smtp.gmail.com" }
            val portStr = editSmtpPort.text.toString().trim()
            val port = portStr.toIntOrNull() ?: 587
            val username = editSmtpUsername.text.toString().trim()
            val password = editSmtpPassword.text.toString().trim()
            val recipient = editSmtpRecipient.text.toString().trim()

            if (port < 1 || port > 65535) {
                showEmailStatus("Cổng SMTP không hợp lệ (1 - 65535)", isError = true)
                return@setOnClickListener
            }
            if (username.isEmpty() || !username.contains("@")) {
                showEmailStatus("Vui lòng nhập Gmail gửi trước khi gửi thử", isError = true)
                return@setOnClickListener
            }
            if (password.isEmpty()) {
                showEmailStatus("Vui lòng nhập Mật khẩu ứng dụng trước khi gửi thử", isError = true)
                return@setOnClickListener
            }
            if (recipient.isEmpty() || !recipient.contains("@")) {
                showEmailStatus("Vui lòng nhập Gmail người nhận trước khi gửi thử", isError = true)
                return@setOnClickListener
            }

            btnTestEmail.isEnabled = false
            showEmailStatus("Đang kết nối SMTP & gửi email thử nghiệm...", isError = false, isPending = true)

            val config = SmtpClient.SmtpConfig(
                host = host,
                port = port,
                username = username,
                password = password,
                recipient = recipient,
                enabled = true
            )

            emailExecutor.execute {
                try {
                    val timeStr = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault()).format(Date())
                    val subject = "[WatchCam] Thử Nghiệm Kết Nối Email SMTP Thành Công"
                    val bodyHtml = """
                        <h2>Kết Nối Gmail SMTP Thành Công</h2>
                        <p>Hệ thống camera an ninh <b>WatchCam</b> đã kết nối thành công tới máy chủ SMTP Gmail.</p>
                        <p>Cấu hình gửi email cảnh báo kèm video clip của bạn đã hoạt động chính xác.</p>
                        <p>Thời gian gửi thử: <b>$timeStr</b></p>
                        <hr/>
                        <small>Email được gửi tự động từ ứng dụng WatchCam trên máy tính bảng.</small>
                    """.trimIndent()

                    val result = SmtpClient.sendEmail(config, subject, bodyHtml, emptyList())
                    runOnUiThread {
                        btnTestEmail.isEnabled = true
                        if (result.isSuccess) {
                            showEmailStatus("Gửi thử email thành công! Vui lòng kiểm tra hộp thư $recipient", isError = false)
                            Toast.makeText(this@MainActivity, "Gửi thử email thành công!", Toast.LENGTH_SHORT).show()
                        } else {
                            val err = result.exceptionOrNull()?.message ?: "Lỗi kết nối SMTP"
                            showEmailStatus("Thử email thất bại: $err", isError = true)
                        }
                    }
                } catch (e: Exception) {
                    runOnUiThread {
                        btnTestEmail.isEnabled = true
                        showEmailStatus("Lỗi: ${e.message}", isError = true)
                    }
                }
            }
        }
    }

    private fun checkAndRequestPermissions() {
        val permissions = mutableListOf(
            Manifest.permission.CAMERA,
            Manifest.permission.RECORD_AUDIO
        )
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            permissions.add(Manifest.permission.POST_NOTIFICATIONS)
        }

        val missingPermissions = permissions.filter {
            ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
        }

        if (missingPermissions.isEmpty()) {
            layoutPermission.visibility = View.GONE
            if (!SurveillanceService.isRunning) {
                startSurveillanceService()
            }
        } else {
            layoutPermission.visibility = View.VISIBLE
            permissionLauncher.launch(missingPermissions.toTypedArray())
        }
    }

    private fun startSurveillanceService() {
        val serviceIntent = Intent(this, SurveillanceService::class.java).apply {
            action = SurveillanceService.ACTION_START
            putExtra(SurveillanceService.EXTRA_PORT, serverPort)
        }
        ContextCompat.startForegroundService(this, serviceIntent)
        updateUiState(true)

        // Try to attach preview shortly after service starts
        uiHandler.postDelayed({
            SurveillanceService.activeCameraManager?.let { cam ->
                cam.attachPreview(cameraPreviewView)
                cam.onCameraSwitched = { isFront ->
                    runOnUiThread {
                        textCameraName.text = if (isFront) "Camera Trước" else "Camera Sau"
                    }
                }
            }
        }, 300)
    }

    private fun stopSurveillanceService() {
        val serviceIntent = Intent(this, SurveillanceService::class.java).apply {
            action = SurveillanceService.ACTION_STOP
        }
        startService(serviceIntent)
        updateUiState(false)
    }

    private fun updateUiState(active: Boolean) {
        if (active) {
            chipStatus.text = getString(R.string.status_monitoring)
            chipStatus.setTextColor(ContextCompat.getColor(this, R.color.accent_green))
            btnToggleSurveillance.text = getString(R.string.btn_stop_surveillance)
            btnToggleSurveillance.setBackgroundColor(ContextCompat.getColor(this, R.color.accent_red))
        } else {
            chipStatus.text = getString(R.string.status_stopped)
            chipStatus.setTextColor(ContextCompat.getColor(this, R.color.accent_amber))
            btnToggleSurveillance.text = getString(R.string.btn_start_surveillance)
            btnToggleSurveillance.setBackgroundColor(ContextCompat.getColor(this, R.color.accent_green))
            textPreviewFps.text = "0.0 FPS"
        }
        updateNetworkUrlDisplay()
    }

    private fun updateNetworkUrlDisplay() {
        val endpoints = NetworkHelper.getAllEndpoints()
        val primaryEndpoint = endpoints.firstOrNull { it.label == "Wi-Fi" } ?: endpoints.firstOrNull()
        val displayIp = primaryEndpoint?.ip ?: NetworkHelper.getLocalIpAddress()
        textServerUrl.text = "Wi-Fi: http://$displayIp:$serverPort | https://$displayIp:8443"
    }

    private fun formatPairingCode(code: String): String {
        return if (code.length == 6) {
            "${code.substring(0, 3)} ${code.substring(3)}"
        } else {
            code
        }
    }

    private fun updateStatusFromNative() {
        if (!SurveillanceService.isRunning) return

        try {
            val jsonStr = WatchCamNative.getStatusJson()
            if (jsonStr.isNotEmpty() && jsonStr != "{}") {
                val json = JSONObject(jsonStr)
                val fps = json.optDouble("actualFps", 0.0)
                val viewers = json.optInt("viewerCount", 0)
                val totalEvents = json.optInt("totalEvents", 0)
                val pairingCode = json.optString("pairingCode", "")
                val camName = json.optString("cameraName", "Camera Sau")
                val resolution = json.optString("resolution", "480p")

                textPreviewFps.text = String.format("%.1f FPS (%s)", fps, resolution.uppercase())
                textViewersCount.text = getString(R.string.label_viewers, viewers)
                textTotalEvents.text = "Sự kiện: $totalEvents"
                textCameraName.text = camName

                if (pairingCode.isNotEmpty()) {
                    textPairingCode.text = formatPairingCode(pairingCode)
                }

                if (json.has("motionEnabled")) {
                    val motionEnabled = json.optBoolean("motionEnabled", true)
                    if (switchMotionDetection.isChecked != motionEnabled) {
                        switchMotionDetection.setOnCheckedChangeListener(null)
                        switchMotionDetection.isChecked = motionEnabled
                        switchMotionDetection.setOnCheckedChangeListener { _, isChecked ->
                            WatchCamNative.updateConfig(
                                isChecked,
                                seekSensitivity.progress,
                                0
                            )
                        }
                    }
                }

                if (json.has("sensitivity")) {
                    val sensitivity = json.optInt("sensitivity", 70)
                    if (!seekSensitivity.isPressed && seekSensitivity.progress != sensitivity) {
                        seekSensitivity.progress = sensitivity
                        textSensitivityLabel.text = getString(R.string.label_sensitivity, sensitivity)
                    }
                }

                // Synchronize email config if changed externally (e.g. from web dashboard)
                val isUserEditingEmail = editSmtpUsername.hasFocus() ||
                        editSmtpPassword.hasFocus() ||
                        editSmtpRecipient.hasFocus() ||
                        editSmtpHost.hasFocus() ||
                        editSmtpPort.hasFocus()

                if (!isUserEditingEmail) {
                    val prefs = getSharedPreferences("watchcam_prefs", Context.MODE_PRIVATE)
                    val editor = prefs.edit()
                    var prefsChanged = false

                    if (json.has("emailAlertsEnabled")) {
                        val remoteEnabled = json.optBoolean("emailAlertsEnabled", false)
                        if (switchEmailAlerts.isChecked != remoteEnabled) {
                            switchEmailAlerts.setOnCheckedChangeListener(null)
                            switchEmailAlerts.isChecked = remoteEnabled
                            switchEmailAlerts.setOnCheckedChangeListener { _, isChecked ->
                                onEmailSwitchChanged(isChecked)
                            }
                        }
                        if (prefs.getBoolean("email_alerts_enabled", false) != remoteEnabled) {
                            editor.putBoolean("email_alerts_enabled", remoteEnabled)
                            prefsChanged = true
                        }
                    }
                    val remoteHost = json.optString("smtpHost", "")
                    if (remoteHost.isNotEmpty()) {
                        if (editSmtpHost.text.toString() != remoteHost) {
                            editSmtpHost.setText(remoteHost)
                        }
                        if (prefs.getString("smtp_host", "") != remoteHost) {
                            editor.putString("smtp_host", remoteHost)
                            prefsChanged = true
                        }
                    }
                    val remotePort = json.optInt("smtpPort", 0)
                    if (remotePort > 0) {
                        if (editSmtpPort.text.toString() != remotePort.toString()) {
                            editSmtpPort.setText(remotePort.toString())
                        }
                        if (prefs.getInt("smtp_port", 587) != remotePort) {
                            editor.putInt("smtp_port", remotePort)
                            prefsChanged = true
                        }
                    }
                    val remoteUser = json.optString("smtpUsername", "")
                    if (remoteUser.isNotEmpty()) {
                        if (editSmtpUsername.text.toString() != remoteUser) {
                            editSmtpUsername.setText(remoteUser)
                        }
                        if (prefs.getString("smtp_username", "") != remoteUser) {
                            editor.putString("smtp_username", remoteUser)
                            prefsChanged = true
                        }
                    }
                    val remotePassword = json.optString("smtpPassword", "")
                    if (remotePassword.isNotEmpty()) {
                        if (editSmtpPassword.text.toString() != remotePassword) {
                            editSmtpPassword.setText(remotePassword)
                        }
                        if (prefs.getString("smtp_password", "") != remotePassword) {
                            editor.putString("smtp_password", remotePassword)
                            prefsChanged = true
                        }
                    }
                    val remoteRecipient = json.optString("smtpRecipient", "")
                    if (remoteRecipient.isNotEmpty()) {
                        if (editSmtpRecipient.text.toString() != remoteRecipient) {
                            editSmtpRecipient.setText(remoteRecipient)
                        }
                        if (prefs.getString("smtp_recipient", "") != remoteRecipient) {
                            editor.putString("smtp_recipient", remoteRecipient)
                            prefsChanged = true
                        }
                    }
                    if (prefsChanged) {
                        editor.apply()
                    }
                }
            }
        } catch (_: Exception) {}
    }

    private fun updateDeviceHealthDisplay() {
        try {
            val telemetry = SurveillanceService.latestTelemetry ?: DeviceTelemetryManager.gatherTelemetry(
                this,
                SurveillanceService.serviceStartTimeMillis
            )

            // 1. Battery
            chipBattery.text = "Pin: ${telemetry.battery.level}% (${telemetry.battery.chargingSource})"
            textBatterySub.text = "Tình trạng: ${telemetry.battery.health}"
            val batteryColor = when {
                telemetry.battery.level <= 15 -> ContextCompat.getColor(this, R.color.accent_red)
                telemetry.battery.level <= 30 -> ContextCompat.getColor(this, R.color.accent_amber)
                else -> ContextCompat.getColor(this, R.color.accent_green)
            }
            chipBattery.setTextColor(batteryColor)

            // 2. Thermal
            chipThermal.text = String.format(Locale.US, "Máy: %.1f°C", telemetry.thermal.temperatureCelsius)
            textThermalSub.text = "Mức nhiệt: ${telemetry.thermal.status}"
            val thermalColor = when (telemetry.thermal.status) {
                "Quá nhiệt" -> ContextCompat.getColor(this, R.color.accent_red)
                "Ấm" -> ContextCompat.getColor(this, R.color.accent_amber)
                else -> ContextCompat.getColor(this, R.color.accent_green)
            }
            chipThermal.setTextColor(thermalColor)
            textHealthSummary.text = telemetry.thermal.status
            textHealthSummary.setTextColor(thermalColor)

            // 3. RAM
            val usedRam = DeviceTelemetryManager.formatBytes(telemetry.ram.usedBytes)
            val totalRam = DeviceTelemetryManager.formatBytes(telemetry.ram.totalBytes)
            chipRam.text = "RAM: $usedRam / $totalRam"
            textRamSub.text = "Đã dùng: ${telemetry.ram.usedPercent}% (Trống: ${DeviceTelemetryManager.formatBytes(telemetry.ram.availableBytes)})"

            // 4. Storage
            val safeTotalBytes = maxOf(telemetry.storage.totalBytes, telemetry.storage.freeBytes)
            val freeStorage = DeviceTelemetryManager.formatBytes(telemetry.storage.freeBytes)
            val totalStorage = DeviceTelemetryManager.formatBytes(safeTotalBytes)
            val mediaStorage = DeviceTelemetryManager.formatBytes(telemetry.storage.snapshotsBytes)
            chipStorage.text = "Trống: $freeStorage"
            textStorageSub.text = "Tổng: $totalStorage (Ảnh & Video: $mediaStorage)"

            // 5. Wi-Fi
            if (telemetry.wifi.connected) {
                chipWifi.text = "Wi-Fi: ${telemetry.wifi.rssi} dBm (${telemetry.wifi.signalQuality})"
                val wifiColor = when (telemetry.wifi.signalQuality) {
                    "Xuất sắc", "Tốt" -> ContextCompat.getColor(this, R.color.accent_blue)
                    "Khá" -> ContextCompat.getColor(this, R.color.accent_amber)
                    else -> ContextCompat.getColor(this, R.color.accent_amber)
                }
                chipWifi.setTextColor(wifiColor)
            } else {
                chipWifi.text = "Wi-Fi: Không kết nối"
                chipWifi.setTextColor(ContextCompat.getColor(this, R.color.accent_red))
            }

            // 6. Uptime
            val uptimeStr = if (SurveillanceService.isRunning) {
                DeviceTelemetryManager.formatDuration(telemetry.uptime.serviceUptimeSeconds)
            } else {
                DeviceTelemetryManager.formatDuration(telemetry.uptime.deviceUptimeSeconds)
            }
            val uptimeLabel = if (SurveillanceService.isRunning) "Giám sát: " else "Máy: "
            chipUptime.text = "$uptimeLabel$uptimeStr"
        } catch (_: Exception) {}
    }

    override fun onResume() {
        super.onResume()
        updateNetworkUrlDisplay()
        updateDeviceHealthDisplay()

        val isRunning = SurveillanceService.isRunning
        updateUiState(isRunning)

        if (isRunning) {
            SurveillanceService.activeCameraManager?.let { cam ->
                cam.attachPreview(cameraPreviewView)
                cam.onCameraSwitched = { isFront ->
                    runOnUiThread {
                        textCameraName.text = if (isFront) "Camera Trước" else "Camera Sau"
                    }
                }
            }
        }

        SurveillanceService.onStatusListener = { active ->
            runOnUiThread {
                updateUiState(active)
                if (active) {
                    SurveillanceService.activeCameraManager?.attachPreview(cameraPreviewView)
                }
            }
        }

        uiHandler.post(statusUpdateRunnable)
    }

    override fun onPause() {
        super.onPause()
        SurveillanceService.activeCameraManager?.detachPreview()
        SurveillanceService.onStatusListener = null
        uiHandler.removeCallbacks(statusUpdateRunnable)
    }

    override fun onDestroy() {
        uiHandler.removeCallbacks(statusUpdateRunnable)
        emailExecutor.shutdown()
        super.onDestroy()
    }
}
