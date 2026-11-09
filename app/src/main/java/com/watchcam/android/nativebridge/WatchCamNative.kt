package com.watchcam.android.nativebridge

import java.nio.ByteBuffer

object WatchCamNative {
    /**
     * Initializes the Rust surveillance engine and starts embedded HTTP/WebSocket server.
     */
    external fun initServer(storageDir: String, dbPath: String, port: Int): Boolean

    /**
     * Stops the surveillance engine and releases resources.
     */
    external fun stopServer()

    /**
     * Processes full YUV_420_888 planes for true color streaming and motion analysis.
     * Returns FloatArray of length 12:
     * [0] -> 1.0f if alert triggered, 0.0f otherwise
     * [1] -> motion score (0.0 .. 1.0)
     * [2] -> 1.0f if camera switch was requested from web dashboard, 0.0f otherwise
     * [3] -> 1.0f if surveillance monitoring is active, 0.0f if stopped
     * [4] -> resolution change code (0 = none, 1 = 480p, 2 = 720p)
     * [5] -> 1.0f if test email was requested from web dashboard, 0.0f otherwise
     * [6] -> entity code (1.0 = person, 2.0 = object, 3.0 = motion, 0.0 = none)
     * [7] -> confidence score (0.0 .. 1.0)
     * [8] -> bounding box x (0.0 .. 1.0)
     * [9] -> bounding box y (0.0 .. 1.0)
     * [10] -> bounding box w (0.0 .. 1.0)
     * [11] -> bounding box h (0.0 .. 1.0)
     */
    external fun processFrameYuv(
        yBuffer: ByteBuffer,
        uBuffer: ByteBuffer,
        vBuffer: ByteBuffer,
        width: Int,
        height: Int,
        yRowStride: Int,
        uRowStride: Int,
        vRowStride: Int,
        uPixelStride: Int,
        vPixelStride: Int,
        rotation: Int
    ): FloatArray

    /**
     * Processes a direct ByteBuffer Y-plane frame.
     * Returns FloatArray of length 5 or 6:
     * [0] -> 1.0f if alert triggered, 0.0f otherwise
     * [1] -> motion score (0.0 .. 1.0)
     * [2] -> 1.0f if camera switch was requested from web dashboard, 0.0f otherwise
     * [3] -> 1.0f if surveillance monitoring is active, 0.0f if stopped
     * [4] -> resolution change code (0 = none, 1 = 480p, 2 = 720p)
     * [5] -> 1.0f if test email was requested from web dashboard, 0.0f otherwise
     */
    external fun processFrame(
        buffer: ByteBuffer,
        width: Int,
        height: Int,
        rowStride: Int,
        rotation: Int
    ): FloatArray

    /**
     * Returns current active 6-digit pairing code.
     */
    external fun getPairingCode(): String

    /**
     * Generates and returns a fresh 6-digit pairing code.
     */
    external fun regeneratePairingCode(): String

    /**
     * Revokes all active authenticated viewer sessions.
     */
    external fun revokeAllSessions()

    /**
     * Revokes all active viewer sessions and guarantees paired tokens are purged from the SQLite database.
     */
    external fun revokeAllSessionsWithDb(dbPath: String)

    /**
     * Updates native surveillance engine configuration.
     */
    external fun updateConfig(motionEnabled: Boolean, sensitivity: Int, streamQuality: Int)

    /**
     * Updates native surveillance engine email configuration.
     * Broadcasts real-time config_updated event over WebSocket.
     */
    external fun updateEmailConfig(
        enabled: Boolean,
        host: String,
        port: Int,
        username: String,
        password: String,
        recipient: String
    )

    /**
     * Informs native engine of the active camera (front vs rear).
     */
    external fun setCameraId(isFront: Boolean)

    /**
     * Sets monitoring state directly.
     */
    external fun setMonitoring(enabled: Boolean)

    /**
     * Checks if monitoring is currently active.
     */
    external fun isMonitoring(): Boolean

    /**
     * Returns JSON string with system status.
     */
    external fun getStatusJson(): String

    /**
     * Returns JSON string of latest motion event.
     */
    external fun getLatestEventJson(): String

    /**
     * Updates native surveillance engine with device hardware health & telemetry status JSON.
     * Broadcasts real-time updates over WebSocket (/ws/events) and exposes in /api/status.
     */
    external fun updateDeviceStatus(statusJson: String): Boolean

    /**
     * Returns JSON string of latest device status.
     */
    external fun getDeviceStatusJson(): String

    /**
     * Notifies native engine that a motion video clip has been recorded.
     * Updates database and broadcasts WebSocket event to connected web clients.
     */
    external fun notifyVideoRecorded(eventId: String, videoPath: String): Boolean

    /**
     * Pushes captured microphone PCM audio frame to native broadcaster.
     * @param data 16-bit PCM little-endian byte array
     * @param length Number of valid bytes in buffer
     * @param level Normalized sound level (0 - 100)
     */
    external fun pushAudioFrame(data: ByteArray, length: Int, level: Int)

    /**
     * Active AudioPlaybackManager for two-way audio playback from web clients.
     */
    @Volatile
    var audioPlaybackManager: com.watchcam.android.audio.AudioPlaybackManager? = null

    /**
     * JNI bridge callback invoked when a client transmits audio over WebSocket (/ws/audio).
     */
    @JvmStatic
    fun receiveClientAudio(data: ByteArray) {
        try {
            audioPlaybackManager?.playPcmChunk(data, 0, data.size)
        } catch (e: Exception) {
            android.util.Log.w("WatchCamNative", "Error playing client intercom audio", e)
        }
    }

    /**
     * Alias for receiveClientAudio to support various JNI callers.
     */
    @JvmStatic
    fun onClientAudioReceived(data: ByteArray) {
        receiveClientAudio(data)
    }
}
