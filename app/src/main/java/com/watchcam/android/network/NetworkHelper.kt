package com.watchcam.android.network

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import java.net.Inet4Address
import java.net.NetworkInterface
import java.util.Collections

object NetworkHelper {

    data class NetworkEndpoint(
        val label: String,
        val ip: String
    )

    /**
     * Identifies virtual, VPN, cellular data, or tunnel network interfaces that must be excluded.
     */
    fun isIgnoredInterface(name: String): Boolean {
        val lower = name.lowercase()
        return lower.startsWith("tun") ||
                lower.startsWith("tailscale") ||
                lower.startsWith("ppp") ||
                lower.startsWith("p2p") ||
                lower.startsWith("wg") ||
                lower.startsWith("tap") ||
                lower.startsWith("rmnet") ||
                lower.startsWith("ccmni") ||
                lower.startsWith("dummy") ||
                lower.startsWith("sit") ||
                lower.startsWith("ip6tnl") ||
                lower.startsWith("docker") ||
                lower.startsWith("veth") ||
                lower.startsWith("virbr")
    }

    /**
     * Checks if the given IP address is a valid local network IPv4 address (RFC 1918 site-local).
     * Excludes loopback (127.0.0.0/8), Tailscale CGNAT (100.64.0.0/10), and link-local (169.254.0.0/16).
     */
    fun isValidLocalIpv4(addr: java.net.InetAddress): Boolean {
        if (addr.isLoopbackAddress || addr.isLinkLocalAddress || addr !is Inet4Address) {
            return false
        }
        val host = addr.hostAddress ?: return false
        if (host.startsWith("127.") || host.startsWith("100.") || host.startsWith("169.254.")) {
            return false
        }
        return addr.isSiteLocalAddress || host.startsWith("192.168.") || host.startsWith("10.")
    }

    /**
     * Enumerates active non-loopback local network endpoints (Wi-Fi, Ethernet).
     * Strictly excludes Tailscale and VPN virtual network interfaces (tun*, tailscale*, 100.64.0.0/10).
     */
    fun getAllEndpoints(): List<NetworkEndpoint> {
        val list = mutableListOf<NetworkEndpoint>()
        try {
            val interfaces = Collections.list(NetworkInterface.getNetworkInterfaces())
            for (intf in interfaces) {
                if (!intf.isUp || intf.isLoopback) continue
                val name = intf.name.lowercase()
                if (isIgnoredInterface(name)) continue
                for (addr in Collections.list(intf.inetAddresses)) {
                    if (isValidLocalIpv4(addr)) {
                        val host = addr.hostAddress ?: continue
                        val label = when {
                            name.startsWith("wlan") -> "Wi-Fi"
                            name.startsWith("eth") -> "Ethernet"
                            else -> "Wi-Fi"
                        }
                        if (list.none { it.ip == host }) {
                            list.add(NetworkEndpoint(label, host))
                        }
                    }
                }
            }
        } catch (e: Exception) {
            e.printStackTrace()
        }
        return list
    }

    /**
     * Finds the primary local IPv4 address of this Android device (excluding VPN and Tailscale).
     */
    fun getLocalIpAddress(): String {
        try {
            val interfaces = Collections.list(NetworkInterface.getNetworkInterfaces())
            // First priority: wlan (Wi-Fi)
            for (intf in interfaces) {
                if (!intf.isUp || intf.isLoopback) continue
                val name = intf.name.lowercase()
                if (isIgnoredInterface(name)) continue
                if (name.startsWith("wlan")) {
                    for (addr in Collections.list(intf.inetAddresses)) {
                        if (isValidLocalIpv4(addr)) {
                            return addr.hostAddress ?: continue
                        }
                    }
                }
            }

            // Second priority: eth (Ethernet)
            for (intf in interfaces) {
                if (!intf.isUp || intf.isLoopback) continue
                val name = intf.name.lowercase()
                if (isIgnoredInterface(name)) continue
                if (name.startsWith("eth")) {
                    for (addr in Collections.list(intf.inetAddresses)) {
                        if (isValidLocalIpv4(addr)) {
                            return addr.hostAddress ?: continue
                        }
                    }
                }
            }

            // Fallback: any other valid local IPv4 interface
            for (intf in interfaces) {
                if (!intf.isUp || intf.isLoopback) continue
                val name = intf.name.lowercase()
                if (isIgnoredInterface(name)) continue
                for (addr in Collections.list(intf.inetAddresses)) {
                    if (isValidLocalIpv4(addr)) {
                        return addr.hostAddress ?: continue
                    }
                }
            }
        } catch (e: Exception) {
            e.printStackTrace()
        }
        return "127.0.0.1"
    }

    /**
     * Checks if Wi-Fi or Ethernet is currently connected.
     */
    fun isNetworkConnected(context: Context): Boolean {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager
            ?: return false
        val activeNetwork = cm.activeNetwork ?: return false
        val caps = cm.getNetworkCapabilities(activeNetwork) ?: return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
                caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
    }
}
