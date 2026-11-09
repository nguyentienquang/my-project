package com.watchcam.android.camera

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.util.Log
import java.io.File
import java.nio.ByteBuffer
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class MotionVideoRecorder {
    companion object {
        private const val TAG = "WatchCam_Recorder"
        private const val VIDEO_MIME_TYPE = MediaFormat.MIMETYPE_VIDEO_AVC
        private const val AUDIO_MIME_TYPE = MediaFormat.MIMETYPE_AUDIO_AAC
        private const val AUDIO_SAMPLE_RATE = 16000
    }

    private val isRecording = AtomicBoolean(false)
    private var videoCodec: MediaCodec? = null
    private var audioCodec: MediaCodec? = null
    private var mediaMuxer: MediaMuxer? = null

    private var videoTrackIndex = -1
    private var audioTrackIndex = -1
    private var hasAudio = false
    private var isMuxerStarted = false

    private var videoFrameCount = 0
    private var totalAudioSamples = 0L
    private var startTimeMs = 0L
    private var durationLimitMs = 4000L
    private var targetFile: File? = null
    private var onFinishedCallback: ((File?) -> Unit)? = null
    private var watchdogFuture: ScheduledFuture<*>? = null
    private var selectedColorFormat: Int = MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar

    private val muxerLock = Any()
    private val pendingBuffers = mutableListOf<PendingBuffer>()

    private val videoRecordExecutor = Executors.newSingleThreadExecutor()
    private val audioRecordExecutor = Executors.newSingleThreadExecutor()
    private val watchdogExecutor: ScheduledExecutorService = Executors.newSingleThreadScheduledExecutor()

    private class PendingBuffer(
        val isAudio: Boolean,
        val data: ByteArray,
        val offset: Int,
        val size: Int,
        val presentationTimeUs: Long,
        val flags: Int
    )

    fun isRecording(): Boolean = isRecording.get()
    fun getSelectedColorFormat(): Int = selectedColorFormat

    @Suppress("DEPRECATION")
    private fun selectColorFormat(codec: MediaCodec): Int {
        return try {
            val caps = codec.codecInfo.getCapabilitiesForType(VIDEO_MIME_TYPE)
            for (fmt in caps.colorFormats) {
                if (fmt == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar) {
                    return fmt
                }
            }
            for (fmt in caps.colorFormats) {
                if (fmt == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420SemiPlanar) {
                    return fmt
                }
            }
            caps.colorFormats.firstOrNull() ?: MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar
        } catch (e: Exception) {
            Log.w(TAG, "Error selecting color format, falling back to COLOR_FormatYUV420Planar", e)
            MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar
        }
    }

    fun startRecording(
        width: Int,
        height: Int,
        rotation: Int,
        outputFile: File,
        durationMs: Long = 4000L,
        onFinished: (File?) -> Unit
    ): Boolean {
        if (!isRecording.compareAndSet(false, true)) {
            Log.w(TAG, "Recording already in progress, skipping duplicate trigger")
            return false
        }

        this.targetFile = outputFile
        this.onFinishedCallback = onFinished
        this.durationLimitMs = durationMs
        this.videoFrameCount = 0
        this.totalAudioSamples = 0L
        this.startTimeMs = System.currentTimeMillis()
        this.isMuxerStarted = false
        this.videoTrackIndex = -1
        this.audioTrackIndex = -1
        this.hasAudio = false
        synchronized(muxerLock) {
            pendingBuffers.clear()
        }

        try {
            // 1. Initialize Video Codec (H.264 / AVC)
            val vCodec = MediaCodec.createEncoderByType(VIDEO_MIME_TYPE)
            val colorFormat = selectColorFormat(vCodec)
            this.selectedColorFormat = colorFormat

            val vFormat = MediaFormat.createVideoFormat(VIDEO_MIME_TYPE, width, height).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, colorFormat)
                setInteger(MediaFormat.KEY_BIT_RATE, 1_200_000)
                setInteger(MediaFormat.KEY_FRAME_RATE, 15)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
            }
            vCodec.configure(vFormat, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            vCodec.start()
            this.videoCodec = vCodec

            // 2. Initialize Audio Codec (AAC 16kHz mono)
            try {
                val aCodec = MediaCodec.createEncoderByType(AUDIO_MIME_TYPE)
                val aFormat = MediaFormat.createAudioFormat(AUDIO_MIME_TYPE, AUDIO_SAMPLE_RATE, 1).apply {
                    setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
                    setInteger(MediaFormat.KEY_BIT_RATE, 32000)
                    setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 16384)
                }
                aCodec.configure(aFormat, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                aCodec.start()
                this.audioCodec = aCodec
                this.hasAudio = true
                Log.i(TAG, "AAC audio encoder configured at ${AUDIO_SAMPLE_RATE}Hz mono")
            } catch (ae: Exception) {
                Log.w(TAG, "AAC encoder setup failed, continuing video-only", ae)
                this.hasAudio = false
                this.audioCodec = null
            }

            // 3. Initialize MediaMuxer
            val muxer = MediaMuxer(outputFile.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
            muxer.setOrientationHint(rotation)
            this.mediaMuxer = muxer

            // 4. Watchdog timer to ensure finalize if stopRecording is delayed
            watchdogFuture?.cancel(false)
            watchdogFuture = watchdogExecutor.schedule({
                if (isRecording.get()) {
                    Log.i(TAG, "Watchdog timer expired, finalizing motion recording")
                    stopRecording()
                }
            }, durationMs + 800L, TimeUnit.MILLISECONDS)

            Log.i(TAG, "Started motion clip recording: ${outputFile.name} (${width}x${height}, rot=$rotation, hasAudio=$hasAudio)")
            return true
        } catch (e: Exception) {
            Log.e(TAG, "Failed to start MediaCodec/MediaMuxer recording", e)
            release()
            isRecording.set(false)
            onFinished(null)
            return false
        }
    }

    fun onFrameAvailable(nv12Data: ByteArray, width: Int = 0, height: Int = 0) {
        if (!isRecording.get()) return

        videoRecordExecutor.execute {
            encodeVideoFrame(nv12Data)
        }
    }

    fun onAudioFrameAvailable(pcmData: ByteArray, length: Int) {
        if (!isRecording.get() || !hasAudio) return

        val copy = pcmData.copyOf(length)
        audioRecordExecutor.execute {
            encodeAudioFrame(copy, length)
        }
    }

    private fun encodeVideoFrame(nv12Data: ByteArray) {
        if (!isRecording.get()) return
        val codec = videoCodec ?: return
        val muxer = mediaMuxer ?: return

        try {
            val elapsed = System.currentTimeMillis() - startTimeMs
            if (elapsed >= durationLimitMs) {
                stopRecording()
                return
            }

            val inputIndex = codec.dequeueInputBuffer(10_000)
            if (inputIndex >= 0) {
                val inputBuffer = codec.getInputBuffer(inputIndex)
                if (inputBuffer != null) {
                    inputBuffer.clear()
                    val toCopy = minOf(nv12Data.size, inputBuffer.remaining())
                    inputBuffer.put(nv12Data, 0, toCopy)

                    val ptsUs = videoFrameCount * 1_000_000L / 15
                    codec.queueInputBuffer(inputIndex, 0, toCopy, ptsUs, 0)
                    videoFrameCount++
                }
            }

            drainVideoCodec(codec, muxer, endOfStream = false)
        } catch (e: Exception) {
            Log.e(TAG, "Error encoding video frame", e)
        }
    }

    private fun encodeAudioFrame(pcmData: ByteArray, length: Int) {
        if (!isRecording.get() || !hasAudio) return
        val codec = audioCodec ?: return
        val muxer = mediaMuxer ?: return

        try {
            val inputIndex = codec.dequeueInputBuffer(10_000)
            if (inputIndex >= 0) {
                val inputBuffer = codec.getInputBuffer(inputIndex)
                if (inputBuffer != null) {
                    inputBuffer.clear()
                    val toCopy = minOf(length, inputBuffer.remaining())
                    inputBuffer.put(pcmData, 0, toCopy)

                    val numSamples = toCopy / 2
                    val ptsUs = totalAudioSamples * 1_000_000L / AUDIO_SAMPLE_RATE.toLong()
                    totalAudioSamples += numSamples
                    codec.queueInputBuffer(inputIndex, 0, toCopy, ptsUs, 0)
                }
            }

            drainAudioCodec(codec, muxer, endOfStream = false)
        } catch (e: Exception) {
            Log.e(TAG, "Error encoding audio frame", e)
        }
    }

    private fun checkStartMuxerLocked(muxer: MediaMuxer, forceStart: Boolean = false) {
        if (isMuxerStarted) return
        if (forceStart && videoTrackIndex >= 0 && audioTrackIndex < 0) {
            Log.w(TAG, "Starting muxer in video-only fallback (audio track was not initialized in time)")
            hasAudio = false
        }
        val tracksReady = videoTrackIndex >= 0 && (!hasAudio || audioTrackIndex >= 0)
        if (tracksReady && !isMuxerStarted) {
            muxer.start()
            isMuxerStarted = true
            Log.i(TAG, "MediaMuxer started with videoTrack=$videoTrackIndex, audioTrack=$audioTrackIndex (flushing ${pendingBuffers.size} samples)")

            for (p in pendingBuffers) {
                val track = if (p.isAudio) audioTrackIndex else videoTrackIndex
                if (track >= 0) {
                    val info = MediaCodec.BufferInfo().apply {
                        set(0, p.size, p.presentationTimeUs, p.flags)
                    }
                    try {
                        muxer.writeSampleData(track, ByteBuffer.wrap(p.data, p.offset, p.size), info)
                    } catch (e: Exception) {
                        Log.w(TAG, "Error flushing pending sample to muxer track $track", e)
                    }
                }
            }
            pendingBuffers.clear()
        }
    }

    private fun drainVideoCodec(codec: MediaCodec, muxer: MediaMuxer, endOfStream: Boolean) {
        val bufferInfo = MediaCodec.BufferInfo()
        var emptyPollCount = 0
        val maxEmptyPolls = if (endOfStream) 20 else 1

        while (true) {
            val outputIndex = codec.dequeueOutputBuffer(bufferInfo, 10_000)
            if (outputIndex == MediaCodec.INFO_TRY_AGAIN_LATER) {
                emptyPollCount++
                if (emptyPollCount >= maxEmptyPolls) break
            } else if (outputIndex == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                synchronized(muxerLock) {
                    if (videoTrackIndex < 0) {
                        videoTrackIndex = muxer.addTrack(codec.outputFormat)
                        Log.i(TAG, "Video track added: index $videoTrackIndex")
                        checkStartMuxerLocked(muxer)
                    }
                }
            } else if (outputIndex >= 0) {
                emptyPollCount = 0
                val outputBuffer = codec.getOutputBuffer(outputIndex)
                if ((bufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG) != 0) {
                    bufferInfo.size = 0
                }
                if (outputBuffer != null && bufferInfo.size > 0) {
                    synchronized(muxerLock) {
                        if (isMuxerStarted && videoTrackIndex >= 0) {
                            outputBuffer.position(bufferInfo.offset)
                            outputBuffer.limit(bufferInfo.offset + bufferInfo.size)
                            muxer.writeSampleData(videoTrackIndex, outputBuffer, bufferInfo)
                        } else {
                            // Queue until muxer starts
                            val bytes = ByteArray(bufferInfo.size)
                            outputBuffer.position(bufferInfo.offset)
                            outputBuffer.get(bytes)
                            pendingBuffers.add(
                                PendingBuffer(
                                    isAudio = false,
                                    data = bytes,
                                    offset = 0,
                                    size = bytes.size,
                                    presentationTimeUs = bufferInfo.presentationTimeUs,
                                    flags = bufferInfo.flags
                                )
                            )
                        }
                    }
                }
                codec.releaseOutputBuffer(outputIndex, false)
                if ((bufferInfo.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                    break
                }
            }
        }
    }

    private fun drainAudioCodec(codec: MediaCodec, muxer: MediaMuxer, endOfStream: Boolean) {
        val bufferInfo = MediaCodec.BufferInfo()
        var emptyPollCount = 0
        val maxEmptyPolls = if (endOfStream) 20 else 1

        while (true) {
            val outputIndex = codec.dequeueOutputBuffer(bufferInfo, 10_000)
            if (outputIndex == MediaCodec.INFO_TRY_AGAIN_LATER) {
                emptyPollCount++
                if (emptyPollCount >= maxEmptyPolls) break
            } else if (outputIndex == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                synchronized(muxerLock) {
                    if (audioTrackIndex < 0) {
                        audioTrackIndex = muxer.addTrack(codec.outputFormat)
                        Log.i(TAG, "Audio track added: index $audioTrackIndex")
                        checkStartMuxerLocked(muxer)
                    }
                }
            } else if (outputIndex >= 0) {
                emptyPollCount = 0
                val outputBuffer = codec.getOutputBuffer(outputIndex)
                if ((bufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG) != 0) {
                    bufferInfo.size = 0
                }
                if (outputBuffer != null && bufferInfo.size > 0) {
                    synchronized(muxerLock) {
                        if (isMuxerStarted && audioTrackIndex >= 0) {
                            outputBuffer.position(bufferInfo.offset)
                            outputBuffer.limit(bufferInfo.offset + bufferInfo.size)
                            muxer.writeSampleData(audioTrackIndex, outputBuffer, bufferInfo)
                        } else {
                            val bytes = ByteArray(bufferInfo.size)
                            outputBuffer.position(bufferInfo.offset)
                            outputBuffer.get(bytes)
                            pendingBuffers.add(
                                PendingBuffer(
                                    isAudio = true,
                                    data = bytes,
                                    offset = 0,
                                    size = bytes.size,
                                    presentationTimeUs = bufferInfo.presentationTimeUs,
                                    flags = bufferInfo.flags
                                )
                            )
                        }
                    }
                }
                codec.releaseOutputBuffer(outputIndex, false)
                if ((bufferInfo.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) {
                    break
                }
            }
        }
    }

    fun stopRecording() {
        if (!isRecording.compareAndSet(true, false)) return

        val cb = onFinishedCallback
        onFinishedCallback = null
        watchdogFuture?.cancel(false)
        watchdogFuture = null

        videoRecordExecutor.execute {
            try {
                val vCodec = videoCodec
                val aCodec = audioCodec
                val muxer = mediaMuxer

                // 1. Drain audio EOS on audio executor so pending audio tasks finish without concurrency race
                if (aCodec != null && muxer != null) {
                    try {
                        audioRecordExecutor.submit {
                            try {
                                drainAudioCodec(aCodec, muxer, endOfStream = false)
                                var inputIndex = -1
                                for (i in 0..5) {
                                    inputIndex = aCodec.dequeueInputBuffer(10_000)
                                    if (inputIndex >= 0) break
                                }
                                if (inputIndex >= 0) {
                                    aCodec.queueInputBuffer(inputIndex, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                                }
                                drainAudioCodec(aCodec, muxer, endOfStream = true)
                            } catch (ae: Exception) {
                                Log.w(TAG, "Error draining audio codec EOS", ae)
                            }
                        }.get(1000, TimeUnit.MILLISECONDS)
                    } catch (e: Exception) {
                        Log.w(TAG, "Timeout or error waiting for audio executor to drain", e)
                    }
                }

                // 2. Drain video EOS
                if (vCodec != null && muxer != null) {
                    drainVideoCodec(vCodec, muxer, endOfStream = false)
                    var inputIndex = -1
                    for (i in 0..5) {
                        inputIndex = vCodec.dequeueInputBuffer(10_000)
                        if (inputIndex >= 0) break
                    }
                    if (inputIndex >= 0) {
                        vCodec.queueInputBuffer(inputIndex, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                    }
                    drainVideoCodec(vCodec, muxer, endOfStream = true)
                }

                // 3. Fallback: ensure muxer started if audio track never arrived
                if (muxer != null) {
                    synchronized(muxerLock) {
                        if (!isMuxerStarted && videoTrackIndex >= 0) {
                            checkStartMuxerLocked(muxer, forceStart = true)
                        }
                    }
                }
            } catch (e: Exception) {
                Log.e(TAG, "Error finalizing recording", e)
            } finally {
                release()
                val file = targetFile
                val validFile = if (file != null && file.exists() && file.length() > 1024) {
                    file
                } else {
                    try { file?.delete() } catch (_: Exception) {}
                    null
                }
                Log.i(TAG, "Motion video recording complete: ${validFile?.name}, length=${validFile?.length() ?: 0} bytes")
                cb?.invoke(validFile)
            }
        }
    }

    fun release() {
        try {
            videoCodec?.stop()
            videoCodec?.release()
        } catch (_: Exception) {}
        videoCodec = null

        try {
            audioCodec?.stop()
            audioCodec?.release()
        } catch (_: Exception) {}
        audioCodec = null

        synchronized(muxerLock) {
            try {
                if (isMuxerStarted) {
                    mediaMuxer?.stop()
                }
                mediaMuxer?.release()
            } catch (_: Exception) {}
            mediaMuxer = null
            isMuxerStarted = false
            pendingBuffers.clear()
        }
    }
}
