package com.watchcam.android.audio

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AudioCaptureTest {

    @Test
    fun testCalculateAudioLevelSilence() {
        val silence = ByteArray(1600) { 0 }
        val level = AudioCaptureManager.calculateAudioLevel(silence, silence.size)
        assertEquals(0, level)
    }

    @Test
    fun testCalculateAudioLevelMaxAmplitude() {
        val maxBuffer = ByteArray(1600)
        for (i in 0 until 1600 step 2) {
            maxBuffer[i] = 0xFF.toByte()
            maxBuffer[i + 1] = 0x7F.toByte()
        }
        val level = AudioCaptureManager.calculateAudioLevel(maxBuffer, maxBuffer.size)
        assertEquals(100, level)
    }

    @Test
    fun testCalculateAudioLevelNormalSpeech() {
        val speechBuffer = ByteArray(1600)
        for (i in 0 until 1600 step 2) {
            speechBuffer[i] = 0xD0.toByte()
            speechBuffer[i + 1] = 0x07.toByte() // 2000 in little-endian
        }
        val level = AudioCaptureManager.calculateAudioLevel(speechBuffer, speechBuffer.size)
        assertTrue("Speech level should be between 10 and 25, got $level", level in 10..25)
    }

    @Test
    fun testCalculateAudioLevelNegativeMaxAmplitude() {
        val negBuffer = ByteArray(1600)
        for (i in 0 until 1600 step 2) {
            negBuffer[i] = 0x00.toByte()
            negBuffer[i + 1] = 0x80.toByte() // -32768 in little-endian
        }
        val level = AudioCaptureManager.calculateAudioLevel(negBuffer, negBuffer.size)
        assertEquals(100, level)
    }

    @Test
    fun testCalculateAudioLevelOddLengthSafety() {
        val oddBuffer = ByteArray(1601) { 0x50.toByte() }
        // Should handle odd lengths gracefully without throwing ArrayIndexOutOfBoundsException
        val level = AudioCaptureManager.calculateAudioLevel(oddBuffer, oddBuffer.size)
        assertTrue(level >= 0)
    }

    @Test
    fun testCalculateAudioLevelZeroOrNegativeLength() {
        val buf = ByteArray(1600)
        assertEquals(0, AudioCaptureManager.calculateAudioLevel(buf, 0))
        assertEquals(0, AudioCaptureManager.calculateAudioLevel(buf, -1))
        assertEquals(0, AudioCaptureManager.calculateAudioLevel(buf, 1))
    }
}
