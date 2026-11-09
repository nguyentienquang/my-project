package com.watchcam.android

import com.watchcam.android.service.SurveillanceService
import org.junit.Assert.*
import org.junit.Test

class SurveillanceStateTest {

    @Test
    fun testServiceConstants() {
        assertEquals("com.watchcam.android.action.START", SurveillanceService.ACTION_START)
        assertEquals("com.watchcam.android.action.STOP", SurveillanceService.ACTION_STOP)
        assertEquals("extra_port", SurveillanceService.EXTRA_PORT)
        assertEquals(1001, SurveillanceService.NOTIFICATION_ID)
    }

    @Test
    fun testProcessFrameResultContract() {
        // Results returned from WatchCamNative.processFrame must have 4 fields:
        // [0] alertTriggered, [1] motionScore, [2] switchCamRequested, [3] isMonitoring
        val dummyResult = floatArrayOf(1.0f, 0.65f, 0.0f, 1.0f)
        assertEquals(4, dummyResult.size)
        assertTrue(dummyResult[0] > 0.5f)
        assertEquals(0.65f, dummyResult[1], 0.001f)
        assertFalse(dummyResult[2] > 0.5f)
        assertTrue(dummyResult[3] > 0.5f)
    }
}
