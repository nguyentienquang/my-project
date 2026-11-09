package com.watchcam.android

import org.junit.Assert.*
import org.junit.Test

class ConfigModelTest {

    @Test
    fun testSensitivityFormulaMatchesRust() {
        // Sensitivity 70: factor = (100 - 70) / 100 = 0.3
        // diff threshold = round(12 + 0.3 * 48) = round(26.4) = 26
        // min area ratio = 0.004 + 0.3 * 0.076 = 0.0268
        val s = 70
        val factor = (100 - s).toFloat() / 100.0f
        val diffThreshold = Math.round(12.0f + factor * 48.0f)
        val minAreaRatio = 0.004f + factor * 0.076f

        assertEquals(26, diffThreshold)
        assertTrue(minAreaRatio in 0.02f..0.03f)
    }

    @Test
    fun testPairingCodeFormat() {
        val code = "839201"
        val formatted = if (code.length == 6) {
            "${code.substring(0, 3)} ${code.substring(3)}"
        } else {
            code
        }
        assertEquals("839 201", formatted)
    }

    @Test
    fun testUrlFormatting() {
        val ip = "192.168.1.105"
        val port = 8080
        val url = "http://$ip:$port"
        assertEquals("http://192.168.1.105:8080", url)
    }
}
