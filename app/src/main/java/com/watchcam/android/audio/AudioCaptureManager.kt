package com.watchcam.android.audio

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.util.Log
import com.watchcam.android.nativebridge.WatchCamNative
import java.util.concurrent.atomic.AtomicBoolean

class AudioCaptureManager(private val context: Context) {
    companion object {
        private const val TAG = "WatchCam_AudioCapture"
        const val SAMPLE_RATE = 16000
        const val CHANNEL_CONFIG = AudioFormat.CHANNEL_IN_MONO
        const val AUDIO_FORMAT = AudioFormat.ENCODING_PCM_16BIT
        // 50ms chunk: 16000 samples/sec * 0.05 sec = 800 samples * 2 bytes/sample = 1600 bytes
        const val CHUNK_SIZE = 1600

        /**
         * Calculates peak/RMS sound level normalized to 0 - 100 for live VU audio meter.
         */
        @JvmStatic
        fun calculateAudioLevel(buffer: ByteArray, length: Int): Int {
            if (length <= 1) return 0
            var sumSquares = 0.0
            var numSamples = 0
            val end = minOf(length - 1, buffer.size - 1)
            for (i in 0 until end step 2) {
                val low = buffer[i].toInt() and 0xFF
                val high = buffer[i + 1].toInt()
                val sample = ((high shl 8) or low).toShort().toInt()
                sumSquares += (sample * sample).toDouble()
                numSamples++
            }
            val rms = if (numSamples > 0) Math.sqrt(sumSquares / numSamples) else 0.0
            // Normal human conversation produces RMS around 500 - 3000 out of 32767
            // Apply scaling factor (x 2.5) so that speech is clearly noticeable on the 0-100 gauge
            val normalized = (rms / 32767.0) * 100.0 * 2.5
            return normalized.toInt().coerceIn(0, 100)
        }
    }

    private val isCapturing = AtomicBoolean(false)
    private var audioRecord: AudioRecord? = null
    private var captureThread: Thread? = null
    private var echoCanceler: android.media.audiofx.AcousticEchoCanceler? = null
    private var noiseSuppressor: android.media.audiofx.NoiseSuppressor? = null
    private var gainControl: android.media.audiofx.AutomaticGainControl? = null

    var onAudioFrameCaptured: ((buffer: ByteArray, length: Int, level: Int) -> Unit)? = null

    fun isCapturing(): Boolean = isCapturing.get()

    @SuppressLint("MissingPermission")
    @Synchronized
    fun startCapture(onFrame: ((buffer: ByteArray, length: Int, level: Int) -> Unit)? = null): Boolean {
        if (isCapturing.get()) {
            Log.d(TAG, "Audio capture is already running")
            return true
        }

        if (onFrame != null) {
            this.onAudioFrameCaptured = onFrame
        }

        val minBufSize = AudioRecord.getMinBufferSize(SAMPLE_RATE, CHANNEL_CONFIG, AUDIO_FORMAT)
        val bufferSize = maxOf(minBufSize * 2, 3200)

        // Attempt initialization across available audio sources: VOICE_COMMUNICATION -> CAMCORDER -> MIC -> DEFAULT
        val sources = intArrayOf(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION,
            MediaRecorder.AudioSource.CAMCORDER,
            MediaRecorder.AudioSource.MIC,
            MediaRecorder.AudioSource.DEFAULT
        )

        var record: AudioRecord? = null
        for (source in sources) {
            try {
                val candidate = AudioRecord(
                    source,
                    SAMPLE_RATE,
                    CHANNEL_CONFIG,
                    AUDIO_FORMAT,
                    bufferSize
                )
                if (candidate.state == AudioRecord.STATE_INITIALIZED) {
                    record = candidate
                    // Enable hardware audio effects if available on this session and retain strong references
                    try {
                        if (android.media.audiofx.AcousticEchoCanceler.isAvailable()) {
                            echoCanceler = android.media.audiofx.AcousticEchoCanceler.create(candidate.audioSessionId)?.apply {
                                enabled = true
                            }
                        }
                    } catch (_: Exception) {}
                    try {
                        if (android.media.audiofx.NoiseSuppressor.isAvailable()) {
                            noiseSuppressor = android.media.audiofx.NoiseSuppressor.create(candidate.audioSessionId)?.apply {
                                enabled = true
                            }
                        }
                    } catch (_: Exception) {}
                    try {
                        if (android.media.audiofx.AutomaticGainControl.isAvailable()) {
                            gainControl = android.media.audiofx.AutomaticGainControl.create(candidate.audioSessionId)?.apply {
                                enabled = true
                            }
                        }
                    } catch (_: Exception) {}

                    Log.i(TAG, "AudioRecord initialized successfully with source $source, bufferSize=$bufferSize")
                    break
                } else {
                    candidate.release()
                }
            } catch (e: Exception) {
                Log.w(TAG, "Failed to initialize AudioRecord with source $source", e)
            }
        }

        if (record == null) {
            Log.e(TAG, "Could not initialize AudioRecord on any audio source")
            return false
        }

        try {
            record.startRecording()
            if (record.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
                Log.e(TAG, "AudioRecord failed to start recording (state=${record.recordingState})")
                record.release()
                return false
            }
        } catch (e: Exception) {
            Log.e(TAG, "Exception starting AudioRecord", e)
            try { record.release() } catch (_: Exception) {}
            return false
        }

        this.audioRecord = record
        isCapturing.set(true)

        val thread = Thread({
            android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO)
            recordingLoop(record)
        }, "WatchCam_AudioCapture")
        this.captureThread = thread
        thread.start()

        Log.i(TAG, "Audio capture started at ${SAMPLE_RATE}Hz mono")
        return true
    }

    private fun recordingLoop(record: AudioRecord) {
        val buffer = ByteArray(CHUNK_SIZE)
        while (isCapturing.get()) {
            val bytesRead = record.read(buffer, 0, buffer.size)
            if (bytesRead > 0) {
                val level = calculateAudioLevel(buffer, bytesRead)

                // Push to native WebSocket broadcaster
                WatchCamNative.pushAudioFrame(buffer, bytesRead, level)

                // Notify callback (e.g. video recorder or UI meters)
                try {
                    onAudioFrameCaptured?.invoke(buffer, bytesRead, level)
                } catch (e: Exception) {
                    Log.w(TAG, "Error in onAudioFrameCaptured callback", e)
                }
            } else if (bytesRead == AudioRecord.ERROR_INVALID_OPERATION || bytesRead == AudioRecord.ERROR_BAD_VALUE) {
                Log.e(TAG, "AudioRecord read error code: $bytesRead")
                break
            }
        }
    }

    @Synchronized
    fun stopCapture() {
        if (!isCapturing.compareAndSet(true, false)) return
        Log.i(TAG, "Stopping audio capture...")

        try { echoCanceler?.release() } catch (_: Exception) {}
        echoCanceler = null
        try { noiseSuppressor?.release() } catch (_: Exception) {}
        noiseSuppressor = null
        try { gainControl?.release() } catch (_: Exception) {}
        gainControl = null

        val record = audioRecord
        audioRecord = null

        try {
            record?.stop()
            record?.release()
        } catch (e: Exception) {
            Log.w(TAG, "Error stopping AudioRecord", e)
        }

        try {
            captureThread?.join(500)
        } catch (_: Exception) {}
        captureThread = null
        Log.i(TAG, "Audio capture stopped")
    }

    fun release() {
        stopCapture()
        onAudioFrameCaptured = null
    }
}
