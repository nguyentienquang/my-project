package com.watchcam.android.audio

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.os.Build
import android.util.Log
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class AudioPlaybackManager(private val context: Context) {
    companion object {
        private const val TAG = "WatchCam_AudioPlayback"
        const val SAMPLE_RATE = 16000
        const val CHANNEL_CONFIG = AudioFormat.CHANNEL_OUT_MONO
        const val AUDIO_FORMAT = AudioFormat.ENCODING_PCM_16BIT
        private const val MAX_QUEUE_CHUNKS = 15 // ~450ms maximum latency buffer
        const val VOLUME_GAIN = 1.8f // Software volume boost for clear tablet loudspeaker output

        /**
         * Applies software gain boost with soft limiter curve to eliminate clipping and harsh distortion.
         */
        internal fun applyVolumeBoostAndSoftLimiter(buffer: ByteArray, gain: Float) {
            val end = buffer.size - 1
            for (i in 0 until end step 2) {
                val low = buffer[i].toInt() and 0xFF
                val high = buffer[i + 1].toInt()
                val sample = ((high shl 8) or low).toShort().toFloat()
                var boosted = sample * gain
                if (boosted > 30000f) {
                    boosted = 30000f + (boosted - 30000f) * 0.2f
                } else if (boosted < -30000f) {
                    boosted = -30000f + (boosted + 30000f) * 0.2f
                }
                val clamped = boosted.toInt().coerceIn(-32768, 32767).toShort()
                buffer[i] = (clamped.toInt() and 0xFF).toByte()
                buffer[i + 1] = ((clamped.toInt() shr 8) and 0xFF).toByte()
            }
        }
    }

    private val isPlaying = AtomicBoolean(false)
    private var audioTrack: AudioTrack? = null
    private val playbackLock = Any()
    private val audioQueue = LinkedBlockingQueue<ByteArray>(MAX_QUEUE_CHUNKS + 10)
    private var playbackThread: Thread? = null

    @Synchronized
    fun start(): Boolean {
        if (isPlaying.get() && audioTrack != null) {
            return true
        }

        try {
            // Ensure system media volume is adequately high for tablet loudspeaker
            val audioManager = context.getSystemService(Context.AUDIO_SERVICE) as? AudioManager
            audioManager?.let { am ->
                try {
                    val maxVol = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
                    val currVol = am.getStreamVolume(AudioManager.STREAM_MUSIC)
                    if (currVol < (maxVol * 0.7).toInt()) {
                        am.setStreamVolume(AudioManager.STREAM_MUSIC, (maxVol * 0.85).toInt(), 0)
                    }
                    if (!am.isSpeakerphoneOn) {
                        am.isSpeakerphoneOn = true
                    }
                } catch (_: Exception) {}
            }

            val minBufSize = AudioTrack.getMinBufferSize(SAMPLE_RATE, CHANNEL_CONFIG, AUDIO_FORMAT)
            val bufferSize = maxOf(minBufSize * 4, 16384)

            val track = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                AudioTrack.Builder()
                    .setAudioAttributes(
                        AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_MEDIA)
                            .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                            .build()
                    )
                    .setAudioFormat(
                        AudioFormat.Builder()
                            .setEncoding(AUDIO_FORMAT)
                            .setSampleRate(SAMPLE_RATE)
                            .setChannelMask(CHANNEL_CONFIG)
                            .build()
                    )
                    .setBufferSizeInBytes(bufferSize)
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    .build()
            } else {
                @Suppress("DEPRECATION")
                AudioTrack(
                    AudioManager.STREAM_MUSIC,
                    SAMPLE_RATE,
                    CHANNEL_CONFIG,
                    AUDIO_FORMAT,
                    bufferSize,
                    AudioTrack.MODE_STREAM
                )
            }

            if (track.state != AudioTrack.STATE_INITIALIZED) {
                Log.e(TAG, "AudioTrack failed to initialize (state=${track.state})")
                track.release()
                return false
            }

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
                track.setVolume(1.0f)
            } else {
                @Suppress("DEPRECATION")
                track.setStereoVolume(1.0f, 1.0f)
            }

            track.play()
            this.audioTrack = track
            isPlaying.set(true)

            // Start dedicated playback worker thread with jitter buffer
            val thread = Thread({
                android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_AUDIO)
                playbackLoop(track)
            }, "WatchCam_AudioPlaybackThread")
            this.playbackThread = thread
            thread.start()

            Log.i(TAG, "AudioPlaybackManager started with dedicated jitter queue (rate=${SAMPLE_RATE}Hz, bufferSize=$bufferSize)")
            return true
        } catch (e: Exception) {
            Log.e(TAG, "Exception creating AudioTrack for playback", e)
            return false
        }
    }

    private fun playbackLoop(track: AudioTrack) {
        while (isPlaying.get()) {
            try {
                val chunk = audioQueue.poll(100, TimeUnit.MILLISECONDS) ?: continue
                var offset = 0
                while (offset < chunk.size && isPlaying.get()) {
                    val written = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                        track.write(chunk, offset, chunk.size - offset, AudioTrack.WRITE_BLOCKING)
                    } else {
                        track.write(chunk, offset, chunk.size - offset)
                    }
                    if (written > 0) {
                        offset += written
                    } else if (written < 0) {
                        Log.w(TAG, "AudioTrack write error code: $written")
                        break
                    }
                }
            } catch (e: InterruptedException) {
                break
            } catch (e: Exception) {
                Log.w(TAG, "Exception in audio playback loop", e)
            }
        }
    }

    fun playPcmChunk(data: ByteArray, offset: Int, length: Int) {
        if (length <= 0) return
        if (!isPlaying.get() || audioTrack == null) {
            if (!start()) return
        }

        try {
            val safeOffset = offset.coerceIn(0, data.size)
            val safeEnd = (safeOffset + length).coerceIn(safeOffset, data.size)
            if (safeEnd <= safeOffset) return
            val chunkCopy = data.copyOfRange(safeOffset, safeEnd)
            applyVolumeBoostAndSoftLimiter(chunkCopy, VOLUME_GAIN)

            // Latency management: drop oldest frames if queue exceeds maximum threshold
            while (audioQueue.size >= MAX_QUEUE_CHUNKS) {
                audioQueue.poll()
            }
            audioQueue.offer(chunkCopy)
        } catch (e: Exception) {
            Log.w(TAG, "Error enqueueing PCM audio chunk", e)
        }
    }

    @Synchronized
    fun stop() {
        if (!isPlaying.compareAndSet(true, false)) return
        audioQueue.clear()

        try {
            playbackThread?.interrupt()
            playbackThread?.join(300)
        } catch (_: Exception) {}
        playbackThread = null

        synchronized(playbackLock) {
            val track = audioTrack
            audioTrack = null

            try {
                track?.stop()
                track?.release()
            } catch (e: Exception) {
                Log.w(TAG, "Error stopping AudioTrack", e)
            }
        }
        Log.i(TAG, "AudioPlaybackManager stopped")
    }

    fun release() {
        stop()
    }
}
