package com.watchcam.android.status

import android.app.ActivityManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.os.Build
import android.os.Environment
import android.os.StatFs
import android.os.SystemClock
import org.json.JSONObject
import java.io.File

data class BatteryInfo(
    val level: Int,                  // 0..100%
    val isCharging: Boolean,
    val chargingSource: String,      // "AC", "USB", "Không dây", "Pin", "Đầy pin"
    val health: String,              // "Tốt", "Quá nhiệt", "Hỏng", "Quá áp", "Lạnh", "Bình thường"
    val temperatureCelsius: Float    // °C
)

data class ThermalInfo(
    val temperatureCelsius: Float,
    val status: String               // "Bình thường", "Ấm", "Quá nhiệt"
)

data class RamInfo(
    val usedBytes: Long,
    val totalBytes: Long,
    val availableBytes: Long,
    val usedPercent: Int             // 0..100%
)

data class StorageInfo(
    val freeBytes: Long,
    val totalBytes: Long,
    val snapshotsBytes: Long
)

data class WifiInfo(
    val connected: Boolean,
    val ssid: String,
    val rssi: Int,                   // dBm
    val signalQuality: String,       // "Xuất sắc", "Tốt", "Yếu", "Không kết nối"
    val linkSpeedMbps: Int
)

data class UptimeInfo(
    val deviceUptimeSeconds: Long,
    val serviceUptimeSeconds: Long
)

data class DeviceTelemetry(
    val battery: BatteryInfo,
    val thermal: ThermalInfo,
    val ram: RamInfo,
    val storage: StorageInfo,
    val wifi: WifiInfo,
    val uptime: UptimeInfo,
    val timestamp: Long
)

object DeviceTelemetryManager {

    fun getBatteryInfo(context: Context): BatteryInfo {
        return try {
            val filter = IntentFilter(Intent.ACTION_BATTERY_CHANGED)
            val intent = context.registerReceiver(null, filter)
            if (intent != null) {
                val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
                val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
                val batteryPct = if (level >= 0 && scale > 0) {
                    ((level / scale.toFloat()) * 100).toInt().coerceIn(0, 100)
                } else {
                    100
                }

                val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
                val isCharging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                        status == BatteryManager.BATTERY_STATUS_FULL

                val plugged = intent.getIntExtra(BatteryManager.EXTRA_PLUGGED, -1)
                val chargingSource = when {
                    status == BatteryManager.BATTERY_STATUS_FULL -> "Đầy pin"
                    plugged == BatteryManager.BATTERY_PLUGGED_AC -> "AC"
                    plugged == BatteryManager.BATTERY_PLUGGED_USB -> "USB"
                    plugged == BatteryManager.BATTERY_PLUGGED_WIRELESS -> "Không dây"
                    isCharging -> "Đang sạc"
                    else -> "Pin"
                }

                val health = when (intent.getIntExtra(BatteryManager.EXTRA_HEALTH, BatteryManager.BATTERY_HEALTH_UNKNOWN)) {
                    BatteryManager.BATTERY_HEALTH_GOOD -> "Tốt"
                    BatteryManager.BATTERY_HEALTH_OVERHEAT -> "Quá nhiệt"
                    BatteryManager.BATTERY_HEALTH_DEAD -> "Hỏng"
                    BatteryManager.BATTERY_HEALTH_OVER_VOLTAGE -> "Quá áp"
                    BatteryManager.BATTERY_HEALTH_COLD -> "Lạnh"
                    else -> "Bình thường"
                }

                val tempTenths = intent.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, 0)
                val tempCelsius = if (tempTenths > 0) tempTenths / 10.0f else 28.0f

                BatteryInfo(
                    level = batteryPct,
                    isCharging = isCharging,
                    chargingSource = chargingSource,
                    health = health,
                    temperatureCelsius = tempCelsius
                )
            } else {
                BatteryInfo(100, false, "Pin", "Tốt", 28.0f)
            }
        } catch (_: Exception) {
            BatteryInfo(100, false, "Pin", "Bình thường", 28.0f)
        }
    }

    fun getThermalInfo(batteryTemp: Float): ThermalInfo {
        val status = when {
            batteryTemp >= 45.0f -> "Quá nhiệt"
            batteryTemp >= 38.0f -> "Ấm"
            else -> "Bình thường"
        }
        return ThermalInfo(
            temperatureCelsius = batteryTemp,
            status = status
        )
    }

    fun getRamInfo(context: Context): RamInfo {
        return try {
            val am = context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager
            val memInfo = ActivityManager.MemoryInfo()
            if (am != null) {
                am.getMemoryInfo(memInfo)
            }
            val total = memInfo.totalMem
            val avail = memInfo.availMem
            val used = (total - avail).coerceAtLeast(0L)
            val usedPct = if (total > 0) ((used.toDouble() / total) * 100).toInt().coerceIn(0, 100) else 0

            RamInfo(
                usedBytes = used,
                totalBytes = total,
                availableBytes = avail,
                usedPercent = usedPct
            )
        } catch (_: Exception) {
            RamInfo(0L, 0L, 0L, 0)
        }
    }

    fun getStorageInfo(context: Context): StorageInfo {
        return try {
            val dir = context.filesDir ?: Environment.getDataDirectory()
            val stat = try {
                StatFs(dir.path)
            } catch (_: Exception) {
                StatFs(Environment.getDataDirectory().path)
            }
            val blockSize = stat.blockSizeLong
            val free = stat.availableBlocksLong * blockSize
            val rawTotal = stat.blockCountLong * blockSize
            // Ensure total is at least free to prevent negative used bytes or free > total UI glitches
            val total = maxOf(rawTotal, free)

            val snapshotsDir = File(context.filesDir, "snapshots")
            val snapshotsBytes = getFolderSize(snapshotsDir)

            StorageInfo(
                freeBytes = free,
                totalBytes = total,
                snapshotsBytes = snapshotsBytes
            )
        } catch (_: Exception) {
            StorageInfo(0L, 0L, 0L)
        }
    }

    private fun getFolderSize(dir: File): Long {
        if (!dir.exists()) return 0L
        var size = 0L
        try {
            val files = dir.listFiles() ?: return 0L
            for (f in files) {
                size += if (f.isDirectory) getFolderSize(f) else f.length()
            }
        } catch (_: Exception) {}
        return size
    }

    fun getWifiInfo(context: Context): WifiInfo {
        return try {
            val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            val activeNet = cm?.activeNetwork
            val caps = if (activeNet != null) cm.getNetworkCapabilities(activeNet) else null
            val isWifiConnected = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true

            val wm = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
            val info = wm?.connectionInfo

            val isConnected = isWifiConnected || (info != null && info.networkId != -1) ||
                    (info != null && info.rssi > -127 && info.rssi != 0)

            if (isConnected) {
                var rssi = info?.rssi ?: -127
                if ((rssi <= -127 || rssi == 0) && caps != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    val sig = caps.signalStrength
                    if (sig != Int.MIN_VALUE && sig != 0) {
                        rssi = sig
                    }
                }
                if (rssi <= -127 || rssi == 0) {
                    rssi = -55 // Sensible default when connected but hardware driver masks RSSI
                }

                var linkSpeed = info?.linkSpeed ?: 0
                if (linkSpeed <= 0 && caps != null) {
                    val dnKbps = caps.linkDownstreamBandwidthKbps
                    if (dnKbps > 0) {
                        linkSpeed = dnKbps / 1000
                    }
                }

                var ssid = info?.ssid ?: ""
                if (ssid.startsWith("\"") && ssid.endsWith("\"") && ssid.length >= 2) {
                    ssid = ssid.substring(1, ssid.length - 1)
                }
                if (ssid.isEmpty() || ssid == "<unknown ssid>" || ssid == ".") {
                    ssid = "Đã kết nối Wi-Fi"
                }

                val quality = when {
                    rssi >= -60 -> "Xuất sắc"
                    rssi >= -75 -> "Tốt"
                    rssi >= -85 -> "Khá"
                    else -> "Yếu"
                }

                WifiInfo(
                    connected = true,
                    ssid = ssid,
                    rssi = rssi,
                    signalQuality = quality,
                    linkSpeedMbps = if (linkSpeed > 0) linkSpeed else 0
                )
            } else {
                WifiInfo(
                    connected = false,
                    ssid = "Không kết nối",
                    rssi = -100,
                    signalQuality = "Không kết nối",
                    linkSpeedMbps = 0
                )
            }
        } catch (_: Exception) {
            WifiInfo(
                connected = false,
                ssid = "Không xác định",
                rssi = 0,
                signalQuality = "Không xác định",
                linkSpeedMbps = 0
            )
        }
    }

    fun getUptimeInfo(serviceStartTimeMillis: Long): UptimeInfo {
        val devUptimeSec = SystemClock.elapsedRealtime() / 1000L
        val srvUptimeSec = if (serviceStartTimeMillis > 0) {
            ((System.currentTimeMillis() - serviceStartTimeMillis) / 1000L).coerceAtLeast(0L)
        } else {
            0L
        }
        return UptimeInfo(
            deviceUptimeSeconds = devUptimeSec,
            serviceUptimeSeconds = srvUptimeSec
        )
    }

    fun formatBytes(bytes: Long): String {
        if (bytes <= 0) return "0 B"
        val units = arrayOf("B", "KB", "MB", "GB", "TB")
        var b = bytes.toDouble()
        var i = 0
        while (b >= 1024.0 && i < units.size - 1) {
            b /= 1024.0
            i++
        }
        return String.format(java.util.Locale.US, "%.1f %s", b, units[i])
    }

    fun formatDuration(seconds: Long): String {
        if (seconds < 0) return "00:00:00"
        val d = seconds / 86400
        val h = (seconds % 86400) / 3600
        val m = (seconds % 3600) / 60
        val s = seconds % 60
        return if (d > 0) {
            String.format(java.util.Locale.US, "%dd %02d:%02d:%02d", d, h, m, s)
        } else {
            String.format(java.util.Locale.US, "%02d:%02d:%02d", h, m, s)
        }
    }

    fun gatherTelemetry(context: Context, serviceStartTimeMillis: Long): DeviceTelemetry {
        val bat = getBatteryInfo(context)
        val therm = getThermalInfo(bat.temperatureCelsius)
        val ram = getRamInfo(context)
        val stor = getStorageInfo(context)
        val wifi = getWifiInfo(context)
        val uptime = getUptimeInfo(serviceStartTimeMillis)

        return DeviceTelemetry(
            battery = bat,
            thermal = therm,
            ram = ram,
            storage = stor,
            wifi = wifi,
            uptime = uptime,
            timestamp = System.currentTimeMillis()
        )
    }

    fun toJsonString(telemetry: DeviceTelemetry): String {
        return buildString {
            append("{")
            append("\"battery\":{")
            append("\"level\":").append(telemetry.battery.level).append(",")
            append("\"isCharging\":").append(telemetry.battery.isCharging).append(",")
            append("\"chargingSource\":\"").append(escapeJson(telemetry.battery.chargingSource)).append("\",")
            append("\"health\":\"").append(escapeJson(telemetry.battery.health)).append("\",")
            append("\"temperature\":").append(String.format(java.util.Locale.US, "%.1f", telemetry.battery.temperatureCelsius))
            append("},")

            append("\"thermal\":{")
            append("\"temperature\":").append(String.format(java.util.Locale.US, "%.1f", telemetry.thermal.temperatureCelsius)).append(",")
            append("\"status\":\"").append(escapeJson(telemetry.thermal.status)).append("\"")
            append("},")

            append("\"ram\":{")
            append("\"usedBytes\":").append(telemetry.ram.usedBytes).append(",")
            append("\"totalBytes\":").append(telemetry.ram.totalBytes).append(",")
            append("\"availableBytes\":").append(telemetry.ram.availableBytes).append(",")
            append("\"usedPercent\":").append(telemetry.ram.usedPercent)
            append("},")

            append("\"storage\":{")
            append("\"freeBytes\":").append(telemetry.storage.freeBytes).append(",")
            append("\"totalBytes\":").append(telemetry.storage.totalBytes).append(",")
            append("\"snapshotsBytes\":").append(telemetry.storage.snapshotsBytes)
            append("},")

            append("\"wifi\":{")
            append("\"connected\":").append(telemetry.wifi.connected).append(",")
            append("\"ssid\":\"").append(escapeJson(telemetry.wifi.ssid)).append("\",")
            append("\"rssi\":").append(telemetry.wifi.rssi).append(",")
            append("\"signalQuality\":\"").append(escapeJson(telemetry.wifi.signalQuality)).append("\",")
            append("\"linkSpeedMbps\":").append(telemetry.wifi.linkSpeedMbps)
            append("},")

            append("\"uptime\":{")
            append("\"deviceUptimeSeconds\":").append(telemetry.uptime.deviceUptimeSeconds).append(",")
            append("\"serviceUptimeSeconds\":").append(telemetry.uptime.serviceUptimeSeconds)
            append("},")

            append("\"timestamp\":").append(telemetry.timestamp)
            append("}")
        }
    }

    private fun escapeJson(s: String): String {
        return s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n").replace("\r", "\\r")
    }

    fun toJson(telemetry: DeviceTelemetry): JSONObject {
        return JSONObject(toJsonString(telemetry))
    }
}
