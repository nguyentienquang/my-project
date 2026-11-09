package com.watchcam.android

import com.watchcam.android.status.BatteryInfo
import com.watchcam.android.status.DeviceTelemetry
import com.watchcam.android.status.DeviceTelemetryManager
import com.watchcam.android.status.RamInfo
import com.watchcam.android.status.StorageInfo
import com.watchcam.android.status.ThermalInfo
import com.watchcam.android.status.UptimeInfo
import com.watchcam.android.status.WifiInfo
import org.junit.Assert.*
import org.junit.Test

class DeviceTelemetryTest {

    @Test
    fun testThermalStatusClassification() {
        val normal = DeviceTelemetryManager.getThermalInfo(30.0f)
        assertEquals("Bình thường", normal.status)
        assertEquals(30.0f, normal.temperatureCelsius, 0.01f)

        val warm = DeviceTelemetryManager.getThermalInfo(38.5f)
        assertEquals("Ấm", warm.status)

        val overheat = DeviceTelemetryManager.getThermalInfo(46.0f)
        assertEquals("Quá nhiệt", overheat.status)
    }

    @Test
    fun testFormatBytes() {
        assertEquals("0 B", DeviceTelemetryManager.formatBytes(0L))
        assertEquals("500.0 B", DeviceTelemetryManager.formatBytes(500L))
        assertEquals("1.0 KB", DeviceTelemetryManager.formatBytes(1024L))
        assertEquals("1.5 MB", DeviceTelemetryManager.formatBytes((1.5 * 1024 * 1024).toLong()))
        assertEquals("2.8 GB", DeviceTelemetryManager.formatBytes((2.8 * 1024 * 1024 * 1024).toLong()))
    }

    @Test
    fun testFormatDuration() {
        assertEquals("00:00:00", DeviceTelemetryManager.formatDuration(0L))
        assertEquals("00:00:45", DeviceTelemetryManager.formatDuration(45L))
        assertEquals("00:01:30", DeviceTelemetryManager.formatDuration(90L))
        assertEquals("01:01:01", DeviceTelemetryManager.formatDuration(3661L))
        assertEquals("1d 01:00:00", DeviceTelemetryManager.formatDuration(90000L))
    }

    @Test
    fun testTelemetryJsonStringSerialization() {
        val telemetry = DeviceTelemetry(
            battery = BatteryInfo(
                level = 95,
                isCharging = true,
                chargingSource = "AC",
                health = "Tốt",
                temperatureCelsius = 31.5f
            ),
            thermal = ThermalInfo(
                temperatureCelsius = 31.5f,
                status = "Bình thường"
            ),
            ram = RamInfo(
                usedBytes = 1500000000L,
                totalBytes = 3000000000L,
                availableBytes = 1500000000L,
                usedPercent = 50
            ),
            storage = StorageInfo(
                freeBytes = 16000000000L,
                totalBytes = 32000000000L,
                snapshotsBytes = 25000000L
            ),
            wifi = WifiInfo(
                connected = true,
                ssid = "WatchCam_WiFi",
                rssi = -55,
                signalQuality = "Xuất sắc",
                linkSpeedMbps = 144
            ),
            uptime = UptimeInfo(
                deviceUptimeSeconds = 12345L,
                serviceUptimeSeconds = 678L
            ),
            timestamp = 1731000000000L
        )

        val jsonStr = DeviceTelemetryManager.toJsonString(telemetry)

        assertTrue(jsonStr.startsWith("{"))
        assertTrue(jsonStr.endsWith("}"))

        // Battery assertions
        assertTrue(jsonStr.contains("\"battery\":{"))
        assertTrue(jsonStr.contains("\"level\":95"))
        assertTrue(jsonStr.contains("\"isCharging\":true"))
        assertTrue(jsonStr.contains("\"chargingSource\":\"AC\""))
        assertTrue(jsonStr.contains("\"health\":\"Tốt\""))
        assertTrue(jsonStr.contains("\"temperature\":31.5"))

        // Thermal assertions
        assertTrue(jsonStr.contains("\"thermal\":{"))
        assertTrue(jsonStr.contains("\"status\":\"Bình thường\""))

        // RAM assertions
        assertTrue(jsonStr.contains("\"ram\":{"))
        assertTrue(jsonStr.contains("\"usedBytes\":1500000000"))
        assertTrue(jsonStr.contains("\"totalBytes\":3000000000"))
        assertTrue(jsonStr.contains("\"availableBytes\":1500000000"))
        assertTrue(jsonStr.contains("\"usedPercent\":50"))

        // Storage assertions
        assertTrue(jsonStr.contains("\"storage\":{"))
        assertTrue(jsonStr.contains("\"freeBytes\":16000000000"))
        assertTrue(jsonStr.contains("\"totalBytes\":32000000000"))
        assertTrue(jsonStr.contains("\"snapshotsBytes\":25000000"))

        // Wi-Fi assertions
        assertTrue(jsonStr.contains("\"wifi\":{"))
        assertTrue(jsonStr.contains("\"connected\":true"))
        assertTrue(jsonStr.contains("\"ssid\":\"WatchCam_WiFi\""))
        assertTrue(jsonStr.contains("\"rssi\":-55"))
        assertTrue(jsonStr.contains("\"signalQuality\":\"Xuất sắc\""))
        assertTrue(jsonStr.contains("\"linkSpeedMbps\":144"))

        // Uptime assertions
        assertTrue(jsonStr.contains("\"uptime\":{"))
        assertTrue(jsonStr.contains("\"deviceUptimeSeconds\":12345"))
        assertTrue(jsonStr.contains("\"serviceUptimeSeconds\":678"))

        assertTrue(jsonStr.contains("\"timestamp\":1731000000000"))
    }

    @Test
    fun testJsonSpecialCharactersEscaping() {
        val telemetry = DeviceTelemetry(
            battery = BatteryInfo(80, false, "Pin \"Nội bộ\"", "Tốt \n 100%", 25.0f),
            thermal = ThermalInfo(25.0f, "Bình thường"),
            ram = RamInfo(100, 200, 100, 50),
            storage = StorageInfo(500, 1000, 50),
            wifi = WifiInfo(true, "WiFi \\ \"Test\"", -50, "Xuất sắc", 100),
            uptime = UptimeInfo(100, 50),
            timestamp = 1000L
        )

        val jsonStr = DeviceTelemetryManager.toJsonString(telemetry)
        assertTrue(jsonStr.contains("\"chargingSource\":\"Pin \\\"Nội bộ\\\"\""))
        assertTrue(jsonStr.contains("\"health\":\"Tốt \\n 100%\""))
        assertTrue(jsonStr.contains("\"ssid\":\"WiFi \\\\ \\\"Test\\\"\""))
        assertTrue(jsonStr.contains("\"level\":80"))
    }
}
