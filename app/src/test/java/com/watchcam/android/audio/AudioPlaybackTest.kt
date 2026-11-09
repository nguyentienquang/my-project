package com.watchcam.android.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AudioPlaybackTest {

    @Test
    fun testSilenceRemainsSilence() {
        val silence = ByteArray(1600) { 0 }
        AudioPlaybackManager.applyVolumeBoostAndSoftLimiter(silence, 1.8f)
        for (b in silence) {
            assertEquals(0.toByte(), b)
        }
    }

    @Test
    fun testNormalSpeechBoosted() {
        val buffer = ByteArray(2)
        val sample: Short = 1000
        buffer[0] = (sample.toInt() and 0xFF).toByte()
        buffer[1] = ((sample.toInt() shr 8) and 0xFF).toByte()

        AudioPlaybackManager.applyVolumeBoostAndSoftLimiter(buffer, 1.8f)

        val low = buffer[0].toInt() and 0xFF
        val high = buffer[1].toInt()
        val boosted = ((high shl 8) or low).toShort().toInt()

        assertEquals(1800, boosted)
    }

    @Test
    fun testSoftLimiterPreventsClipping() {
        // Input: 20000 -> 20000 * 1.8 = 36000 (exceeds 30000)
        // Soft limiter: 30000 + (36000 - 30000) * 0.2 = 30000 + 1200 = 31200 <= 32767
        val buffer = ByteArray(2)
        val sample: Short = 20000
        buffer[0] = (sample.toInt() and 0xFF).toByte()
        buffer[1] = ((sample.toInt() shr 8) and 0xFF).toByte()

        AudioPlaybackManager.applyVolumeBoostAndSoftLimiter(buffer, 1.8f)

        val low = buffer[0].toInt() and 0xFF
        val high = buffer[1].toInt()
        val result = ((high shl 8) or low).toShort().toInt()

        assertEquals(31200, result)
        assertTrue("Result must not overflow 32767", result <= 32767)
    }

    @Test
    fun testNegativeSoftLimiterPreventsClipping() {
        // Input: -20000 -> -20000 * 1.8 = -36000 (exceeds -30000)
        // Soft limiter: -30000 + (-36000 + 30000) * 0.2 = -30000 - 1200 = -31200 >= -32768
        val buffer = ByteArray(2)
        val sample: Short = -20000
        buffer[0] = (sample.toInt() and 0xFF).toByte()
        buffer[1] = ((sample.toInt() shr 8) and 0xFF).toByte()

        AudioPlaybackManager.applyVolumeBoostAndSoftLimiter(buffer, 1.8f)

        val low = buffer[0].toInt() and 0xFF
        val high = buffer[1].toInt()
        val result = ((high shl 8) or low).toShort().toInt()

        assertEquals(-31200, result)
        assertTrue("Result must not underflow -32768", result >= -32768)
    }

    @Test
    fun testOddLengthHandlingSafe() {
        val oddBuffer = ByteArray(7) { 0x40.toByte() }
        AudioPlaybackManager.applyVolumeBoostAndSoftLimiter(oddBuffer, 1.8f)
        // Passes without exception
        assertTrue(true)
    }

    @Test
    fun testEmptyAndSingleByteBuffersSafe() {
        val empty = ByteArray(0)
        AudioPlaybackManager.applyVolumeBoostAndSoftLimiter(empty, 1.8f)
        val single = ByteArray(1) { 0x55.toByte() }
        AudioPlaybackManager.applyVolumeBoostAndSoftLimiter(single, 1.8f)
        assertEquals(0x55.toByte(), single[0])
    }
}
