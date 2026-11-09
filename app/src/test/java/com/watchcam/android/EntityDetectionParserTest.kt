package com.watchcam.android

import com.watchcam.android.camera.CameraManager
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class EntityDetectionParserTest {

    @Test
    fun testParseInsufficientLength() {
        val shortArray = FloatArray(11)
        val result = CameraManager.parseEntityDetection(shortArray)
        assertNull("Should return null when results array has fewer than 12 elements", result)
    }

    @Test
    fun testParsePersonDetection() {
        val results = FloatArray(12)
        results[0] = 1.0f // alert
        results[1] = 0.85f // score
        results[6] = 1.0f // entityCode 1 = person
        results[7] = 0.92f // confidence
        results[8] = 100f // bx
        results[9] = 50f // by
        results[10] = 200f // bw
        results[11] = 400f // bh

        val result = CameraManager.parseEntityDetection(results)
        assertNotNull(result)
        assertEquals("person", result!!.entityType)
        assertEquals(0.92f, result.confidence, 0.001f)
        assertNotNull(result.boundingBox)
        assertArrayEquals(floatArrayOf(100f, 50f, 200f, 400f), result.boundingBox, 0.001f)
    }

    @Test
    fun testParseObjectDetection() {
        val results = FloatArray(12)
        results[6] = 2.0f // entityCode 2 = object
        results[7] = 0.77f // confidence
        results[8] = 50f
        results[9] = 60f
        results[10] = 120f
        results[11] = 110f

        val result = CameraManager.parseEntityDetection(results)
        assertNotNull(result)
        assertEquals("object", result!!.entityType)
        assertEquals(0.77f, result.confidence, 0.001f)
        assertNotNull(result.boundingBox)
        assertArrayEquals(floatArrayOf(50f, 60f, 120f, 110f), result.boundingBox, 0.001f)
    }

    @Test
    fun testParseMotionDetectionWithoutBox() {
        val results = FloatArray(12)
        results[6] = 3.0f // entityCode 3 = motion
        results[7] = 0.45f
        results[8] = 0f
        results[9] = 0f
        results[10] = 0f // bw is 0
        results[11] = 0f // bh is 0

        val result = CameraManager.parseEntityDetection(results)
        assertNotNull(result)
        assertEquals("motion", result!!.entityType)
        assertEquals(0.45f, result.confidence, 0.001f)
        assertNull("Bounding box should be null if width or height is zero", result.boundingBox)
    }

    @Test
    fun testParseIdleZeroDetectionReturnsNull() {
        val results = FloatArray(12)
        // All zeros: idle state without detection
        val result = CameraManager.parseEntityDetection(results)
        assertNull("Should return null for idle zero frames", result)
    }
}
