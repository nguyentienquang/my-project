package com.watchcam.android.camera

import android.content.Context
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CaptureRequest
import android.media.MediaCodecInfo
import android.util.Log
import android.util.Range
import android.util.Size
import android.view.OrientationEventListener
import android.view.Surface
import android.view.WindowManager
import android.os.Build
import androidx.camera.camera2.interop.Camera2Interop
import androidx.camera.camera2.interop.ExperimentalCamera2Interop
import androidx.camera.core.AspectRatio
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.AspectRatioStrategy
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import com.watchcam.android.nativebridge.WatchCamNative
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

class CameraManager(
    private val context: Context,
    private val lifecycleOwner: LifecycleOwner,
    private var previewView: PreviewView? = null
) {

    data class EntityDetection(
        val entityType: String,
        val confidence: Float,
        val boundingBox: FloatArray?
    )

    companion object {
        private const val TAG = "WatchCam_CameraManager"
        private val TARGET_RESOLUTION = Size(640, 480)

        internal fun parseEntityDetection(results: FloatArray): EntityDetection? {
            if (results.size < 12) return null
            val entityCode = results[6].toInt()
            val confidence = results[7]
            val bbox = if (results[10] > 0f && results[11] > 0f) {
                floatArrayOf(results[8], results[9], results[10], results[11])
            } else null
            if (entityCode <= 0 && confidence <= 0f && bbox == null) {
                return null
            }
            val entityType = when (entityCode) {
                1 -> "person"
                2 -> "object"
                3 -> "motion"
                else -> "motion"
            }
            return EntityDetection(entityType, confidence, bbox)
        }
    }

    private var targetResolution: Size = TARGET_RESOLUTION
    private var lastRotation: Int = 0
    private var currentTargetRotation: Int = getInitialTargetRotation()
    private var cameraProvider: ProcessCameraProvider? = null
    private var orientationEventListener: OrientationEventListener? = null

    private fun getInitialTargetRotation(): Int {
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                context.display?.rotation ?: Surface.ROTATION_0
            } else {
                @Suppress("DEPRECATION")
                (context.getSystemService(Context.WINDOW_SERVICE) as? WindowManager)?.defaultDisplay?.rotation
                    ?: Surface.ROTATION_0
            }
        } catch (e: Exception) {
            Surface.ROTATION_0
        }
    }

    fun getCurrentResolution(): Size = targetResolution
    fun getLastRotation(): Int = lastRotation
    private var camera: Camera? = null
    private var imageAnalysis: ImageAnalysis? = null
    private var preview: Preview? = null

    private val cameraExecutor: ExecutorService = Executors.newSingleThreadExecutor()
    val videoRecorder = MotionVideoRecorder()

    var isUsingFrontCamera: Boolean = false
        private set

    var onMotionDetected: ((isAlert: Boolean, score: Float) -> Unit)? = null
    var onMotionAlertWithFrame: ((score: Float, width: Int, height: Int, rotation: Int) -> Unit)? = null
    var onEntityDetected: ((entityType: String, confidence: Float, bbox: FloatArray?) -> Unit)? = null
    var onTestEmailRequested: (() -> Unit)? = null
    var onCameraSwitched: ((isFront: Boolean) -> Unit)? = null
    var onMonitoringStateChanged: ((isActive: Boolean) -> Unit)? = null
    private var lastMonitoringState: Boolean? = null

    init {
        initCameraProvider()
    }

    private fun initCameraProvider() {
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener({
            try {
                cameraProvider = future.get()
                Log.i(TAG, "ProcessCameraProvider obtained")
            } catch (e: Exception) {
                Log.e(TAG, "Error obtaining CameraProvider", e)
            }
        }, ContextCompat.getMainExecutor(context))
    }

    private fun initOrientationListener() {
        if (orientationEventListener != null) return
        orientationEventListener = object : OrientationEventListener(context) {
            override fun onOrientationChanged(orientation: Int) {
                if (orientation == ORIENTATION_UNKNOWN) return
                val targetRotation = when (orientation) {
                    in 45..134 -> Surface.ROTATION_270
                    in 135..224 -> Surface.ROTATION_180
                    in 225..314 -> Surface.ROTATION_90
                    else -> Surface.ROTATION_0
                }
                if (targetRotation != currentTargetRotation) {
                    currentTargetRotation = targetRotation
                    imageAnalysis?.targetRotation = targetRotation
                    preview?.targetRotation = targetRotation
                }
            }
        }
    }

    /**
     * Starts camera acquisition pipeline with Preview (if available) and ImageAnalysis.
     */
    fun startCamera() {
        initOrientationListener()
        if (orientationEventListener?.canDetectOrientation() == true) {
            orientationEventListener?.enable()
        }
        val currentDisplayRot = getInitialTargetRotation()
        if (currentTargetRotation != currentDisplayRot) {
            currentTargetRotation = currentDisplayRot
            imageAnalysis?.targetRotation = currentDisplayRot
            preview?.targetRotation = currentDisplayRot
        }
        if (cameraProvider == null) {
            val future = ProcessCameraProvider.getInstance(context)
            future.addListener({
                cameraProvider = future.get()
                bindCameraUseCases()
            }, ContextCompat.getMainExecutor(context))
            return
        }
        bindCameraUseCases()
    }

    /**
     * Dynamically attaches a PreviewView from an Activity without restarting ImageAnalysis.
     */
    fun attachPreview(view: PreviewView) {
        ContextCompat.getMainExecutor(context).execute {
            this.previewView = view
            bindCameraUseCases()
        }
    }

    /**
     * Detaches PreviewView when Activity is paused, allowing background ImageAnalysis to continue.
     */
    fun detachPreview() {
        ContextCompat.getMainExecutor(context).execute {
            this.previewView = null
            bindCameraUseCases()
        }
    }

    private fun getBestFpsRange(isFront: Boolean): Range<Int>? {
        return try {
            val cm = context.getSystemService(Context.CAMERA_SERVICE) as? android.hardware.camera2.CameraManager
                ?: return null
            val targetFacing = if (isFront) {
                CameraCharacteristics.LENS_FACING_FRONT
            } else {
                CameraCharacteristics.LENS_FACING_BACK
            }

            for (id in cm.cameraIdList) {
                val chars = cm.getCameraCharacteristics(id)
                if (chars.get(CameraCharacteristics.LENS_FACING) == targetFacing) {
                    val ranges = chars.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES) ?: continue
                    Log.i(TAG, "Available AE FPS ranges for cam $id (isFront=$isFront): ${ranges.contentToString()}")
                    // Prioritize fixed 30fps ([30, 30]) or fixed 20fps ([20, 20])
                    // To maximize throughput while preventing sensor exposure down-throttling in indoor lighting
                    return ranges.find { it.lower == 30 && it.upper == 30 }
                        ?: ranges.find { it.lower == 20 && it.upper == 20 }
                        ?: ranges.filter { it.upper >= 20 }.maxByOrNull { it.lower }
                        ?: ranges.maxByOrNull { it.upper }
                }
            }
            null
        } catch (e: Exception) {
            Log.w(TAG, "Error selecting AE FPS range", e)
            null
        }
    }

    @androidx.annotation.OptIn(ExperimentalCamera2Interop::class)
    private fun bindCameraUseCases() {
        val provider = cameraProvider ?: return

        try {
            provider.unbindAll()

            val cameraSelector = if (isUsingFrontCamera) {
                CameraSelector.DEFAULT_FRONT_CAMERA
            } else {
                CameraSelector.DEFAULT_BACK_CAMERA
            }

            // Check if selected camera is available; fallback if necessary
            val targetSelector = if (provider.hasCamera(cameraSelector)) {
                cameraSelector
            } else {
                CameraSelector.DEFAULT_BACK_CAMERA
            }

            // Match aspect ratio: 16:9 for 1280x720, 4:3 for 640x480
            val aspectStrategy = if (targetResolution.width == 1280 && targetResolution.height == 720) {
                AspectRatioStrategy(
                    AspectRatio.RATIO_16_9,
                    AspectRatioStrategy.FALLBACK_RULE_AUTO
                )
            } else {
                AspectRatioStrategy(
                    AspectRatio.RATIO_4_3,
                    AspectRatioStrategy.FALLBACK_RULE_AUTO
                )
            }

            val resolutionSelector = ResolutionSelector.Builder()
                .setAspectRatioStrategy(aspectStrategy)
                .setResolutionStrategy(
                    ResolutionStrategy(
                        targetResolution,
                        ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER
                    )
                )
                .build()

            val selectedFpsRange = getBestFpsRange(isUsingFrontCamera)
            Log.i(TAG, "Binding camera use cases with resolution $targetResolution, FPS range: $selectedFpsRange")

            // ImageAnalysis UseCase with dynamically selected AE target FPS range
            val analysisBuilder = ImageAnalysis.Builder()
                .setResolutionSelector(resolutionSelector)
                .setTargetRotation(currentTargetRotation)
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_YUV_420_888)

            if (selectedFpsRange != null) {
                Camera2Interop.Extender(analysisBuilder)
                    .setCaptureRequestOption(
                        CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE,
                        selectedFpsRange
                    )
            }

            imageAnalysis = analysisBuilder.build()
                .apply {
                    setAnalyzer(cameraExecutor) { imageProxy ->
                        try {
                            val image = imageProxy.image
                            if (image != null && image.planes.isNotEmpty()) {
                                val planes = image.planes
                                val yPlane = planes[0]
                                val uPlane = if (planes.size > 1) planes[1] else null
                                val vPlane = if (planes.size > 2) planes[2] else null

                                val width = image.width
                                val height = image.height
                                val rotation = imageProxy.imageInfo.rotationDegrees
                                lastRotation = rotation

                                // Full true-color YUV processing over JNI
                                val results = if (uPlane != null && vPlane != null) {
                                    WatchCamNative.processFrameYuv(
                                        yPlane.buffer,
                                        uPlane.buffer,
                                        vPlane.buffer,
                                        width,
                                        height,
                                        yPlane.rowStride,
                                        uPlane.rowStride,
                                        vPlane.rowStride,
                                        uPlane.pixelStride,
                                        vPlane.pixelStride,
                                        rotation
                                    )
                                } else {
                                    WatchCamNative.processFrame(
                                        yPlane.buffer,
                                        width,
                                        height,
                                        yPlane.rowStride,
                                        rotation
                                    )
                                }

                                if (results.size >= 4) {
                                    val alertTriggered = results[0] > 0.5f
                                    val motionScore = results[1]
                                    val switchCamRequested = results[2] > 0.5f
                                    val isMonitoring = results[3] > 0.5f

                                    if (switchCamRequested) {
                                        ContextCompat.getMainExecutor(context).execute {
                                            switchCamera()
                                        }
                                    }

                                    if (results.size >= 5) {
                                        val resCode = results[4].toInt()
                                        if (resCode == 1 && (targetResolution.width != 640 || targetResolution.height != 480)) {
                                            ContextCompat.getMainExecutor(context).execute {
                                                setResolution(640, 480)
                                            }
                                        } else if (resCode == 2 && (targetResolution.width != 1280 || targetResolution.height != 720)) {
                                            ContextCompat.getMainExecutor(context).execute {
                                                setResolution(1280, 720)
                                            }
                                        }
                                    }

                                    if (results.size >= 6 && results[5] > 0.5f) {
                                        onTestEmailRequested?.invoke()
                                    }

                                    val entity = parseEntityDetection(results)
                                    if (entity != null) {
                                        onEntityDetected?.invoke(entity.entityType, entity.confidence, entity.boundingBox)
                                    }

                                    if (alertTriggered) {
                                        onMotionAlertWithFrame?.invoke(motionScore, width, height, rotation)
                                    }

                                    // Feed frame to video recorder if active clip recording
                                    if (videoRecorder.isRecording()) {
                                        val colorFormat = videoRecorder.getSelectedColorFormat()
                                        val frameData = if (colorFormat == MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar) {
                                            extractI420(
                                                yPlane.buffer,
                                                uPlane?.buffer,
                                                vPlane?.buffer,
                                                width,
                                                height,
                                                yPlane.rowStride,
                                                uPlane?.rowStride ?: (width / 2),
                                                vPlane?.rowStride ?: (width / 2),
                                                uPlane?.pixelStride ?: 1,
                                                vPlane?.pixelStride ?: 1
                                            )
                                        } else {
                                            extractNv12(
                                                yPlane.buffer,
                                                uPlane?.buffer,
                                                vPlane?.buffer,
                                                width,
                                                height,
                                                yPlane.rowStride,
                                                uPlane?.rowStride ?: width,
                                                vPlane?.rowStride ?: width,
                                                uPlane?.pixelStride ?: 1,
                                                vPlane?.pixelStride ?: 1
                                            )
                                        }
                                        videoRecorder.onFrameAvailable(frameData, width, height)
                                    }

                                    onMotionDetected?.invoke(alertTriggered, motionScore)
                                    if (lastMonitoringState != isMonitoring) {
                                        lastMonitoringState = isMonitoring
                                        onMonitoringStateChanged?.invoke(isMonitoring)
                                    }
                                }
                            }
                        } catch (e: Exception) {
                            Log.e(TAG, "Frame analysis error", e)
                        } finally {
                            // Non-negotiable: Always close ImageProxy!
                            imageProxy.close()
                        }
                    }
                }

            // Preview UseCase if PreviewView is present
            val currentPreviewView = previewView
            if (currentPreviewView != null) {
                val previewBuilder = Preview.Builder()
                    .setResolutionSelector(resolutionSelector)
                    .setTargetRotation(currentTargetRotation)

                if (selectedFpsRange != null) {
                    Camera2Interop.Extender(previewBuilder)
                        .setCaptureRequestOption(
                            CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE,
                            selectedFpsRange
                        )
                }

                preview = previewBuilder.build()
                    .also {
                        it.setSurfaceProvider(currentPreviewView.surfaceProvider)
                    }

                camera = provider.bindToLifecycle(
                    lifecycleOwner,
                    targetSelector,
                    preview,
                    imageAnalysis
                )
            } else {
                camera = provider.bindToLifecycle(
                    lifecycleOwner,
                    targetSelector,
                    imageAnalysis
                )
            }

            Log.i(TAG, "Camera bound successfully. FrontCamera: $isUsingFrontCamera, HasPreview: ${currentPreviewView != null}")
        } catch (e: Exception) {
            Log.e(TAG, "Failed to bind camera use cases", e)
        }
    }

    /**
     * Toggles between front and rear cameras.
     */
    fun switchCamera() {
        val provider = cameraProvider ?: return
        val nextIsFront = !isUsingFrontCamera
        val nextSelector = if (nextIsFront) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA

        if (provider.hasCamera(nextSelector)) {
            isUsingFrontCamera = nextIsFront
            WatchCamNative.setCameraId(isUsingFrontCamera)
            bindCameraUseCases()
            onCameraSwitched?.invoke(isUsingFrontCamera)
            Log.i(TAG, "Switched camera to isFront: $isUsingFrontCamera")
        } else {
            Log.w(TAG, "Cannot switch: Target camera unavailable on this device")
        }
    }

    /**
     * Changes camera capture resolution dynamically (e.g. 640x480 or 1280x720).
     */
    fun setResolution(width: Int, height: Int) {
        if (targetResolution.width == width && targetResolution.height == height) return
        Log.i(TAG, "Updating target resolution to ${width}x${height}")
        targetResolution = Size(width, height)
        bindCameraUseCases()
    }

    /**
     * Stops camera acquisition and releases all use cases.
     */
    fun stopCamera() {
        try {
            orientationEventListener?.disable()
            cameraProvider?.unbindAll()
            camera = null
            imageAnalysis = null
            preview = null
            previewView = null
            Log.i(TAG, "Camera stopped and use cases unbound")
        } catch (e: Exception) {
            Log.e(TAG, "Error stopping camera", e)
        }
    }

    private fun extractI420(
        yBuffer: java.nio.ByteBuffer,
        uBuffer: java.nio.ByteBuffer?,
        vBuffer: java.nio.ByteBuffer?,
        width: Int,
        height: Int,
        yRowStride: Int,
        uRowStride: Int,
        vRowStride: Int,
        uPixelStride: Int,
        vPixelStride: Int
    ): ByteArray {
        val i420 = ByteArray(width * height * 3 / 2)
        val yPos = yBuffer.position()

        // 1. Copy Y plane
        if (yRowStride == width) {
            yBuffer.get(i420, 0, width * height)
        } else {
            for (row in 0 until height) {
                yBuffer.position(yPos + row * yRowStride)
                yBuffer.get(i420, row * width, width)
            }
        }
        yBuffer.position(yPos)

        // 2. Copy U plane (Planar: Y... then U... then V...)
        val uOffset = width * height
        val vOffset = uOffset + (width * height / 4)
        val halfWidth = width / 2
        val halfHeight = height / 2

        if (uBuffer != null) {
            val uPos = uBuffer.position()
            for (row in 0 until halfHeight) {
                val rowStart = uPos + row * uRowStride
                val dstRow = uOffset + row * halfWidth
                for (col in 0 until halfWidth) {
                    i420[dstRow + col] = uBuffer.get(rowStart + col * uPixelStride)
                }
            }
            uBuffer.position(uPos)
        }

        // 3. Copy V plane
        if (vBuffer != null) {
            val vPos = vBuffer.position()
            for (row in 0 until halfHeight) {
                val rowStart = vPos + row * vRowStride
                val dstRow = vOffset + row * halfWidth
                for (col in 0 until halfWidth) {
                    i420[dstRow + col] = vBuffer.get(rowStart + col * vPixelStride)
                }
            }
            vBuffer.position(vPos)
        }

        return i420
    }

    private fun extractNv12(
        yBuffer: java.nio.ByteBuffer,
        uBuffer: java.nio.ByteBuffer?,
        vBuffer: java.nio.ByteBuffer?,
        width: Int,
        height: Int,
        yRowStride: Int,
        uRowStride: Int,
        vRowStride: Int,
        uPixelStride: Int,
        vPixelStride: Int
    ): ByteArray {
        val nv12 = ByteArray(width * height * 3 / 2)
        val yPos = yBuffer.position()

        // 1. Copy Y plane
        if (yRowStride == width) {
            yBuffer.get(nv12, 0, width * height)
        } else {
            for (row in 0 until height) {
                yBuffer.position(yPos + row * yRowStride)
                yBuffer.get(nv12, row * width, width)
            }
        }
        yBuffer.position(yPos)

        // 2. Interleave U and V plane into NV12 (U0, V0, U1, V1...)
        if (uBuffer != null && vBuffer != null) {
            val uPos = uBuffer.position()
            val vPos = vBuffer.position()
            val halfWidth = width / 2
            val halfHeight = height / 2
            var dstIdx = width * height

            for (row in 0 until halfHeight) {
                val uRowStart = uPos + row * uRowStride
                val vRowStart = vPos + row * vRowStride
                for (col in 0 until halfWidth) {
                    nv12[dstIdx++] = uBuffer.get(uRowStart + col * uPixelStride)
                    nv12[dstIdx++] = vBuffer.get(vRowStart + col * vPixelStride)
                }
            }
            uBuffer.position(uPos)
            vBuffer.position(vPos)
        }

        return nv12
    }

    fun release() {
        orientationEventListener?.disable()
        orientationEventListener = null
        stopCamera()
        videoRecorder.release()
        cameraExecutor.shutdown()
    }
}
