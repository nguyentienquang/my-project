package com.watchcam.android

import com.watchcam.android.network.NetworkHelper
import org.junit.Assert.*
import org.junit.Test

class NetworkHelperTest {

    @Test
    fun testGetLocalIpAddressReturnsNonNull() {
        val ip = NetworkHelper.getLocalIpAddress()
        assertNotNull(ip)
        assertTrue(ip.isNotEmpty())
        // Should be a valid IPv4 string format: x.x.x.x
        val parts = ip.split(".")
        assertEquals(4, parts.size)
        for (part in parts) {
            val num = part.toIntOrNull()
            assertNotNull(num)
            assertTrue(num!! in 0..255)
        }
        // Must never return Tailscale CGNAT IP range (100.x.x.x)
        assertFalse("Local IP must not be a Tailscale CGNAT address", ip.startsWith("100."))
    }

    @Test
    fun testGetAllEndpointsExcludesTailscaleAndVpn() {
        val endpoints = NetworkHelper.getAllEndpoints()
        for (ep in endpoints) {
            assertNotNull(ep.label)
            assertNotNull(ep.ip)
            assertTrue("Endpoint IP must not be empty", ep.ip.isNotEmpty())
            assertFalse("Endpoint must not be labeled Tailscale", ep.label.equals("Tailscale", ignoreCase = true))
            assertFalse("Endpoint must not be labeled VPN", ep.label.equals("VPN", ignoreCase = true))
            assertFalse("Endpoint IP must not be a Tailscale 100.x.x.x address", ep.ip.startsWith("100."))
            assertFalse("Endpoint IP must not be a loopback address", ep.ip.startsWith("127."))
        }
    }

    @Test
    fun testIsIgnoredInterfaceFiltersVpnCellularAndTunnels() {
        assertTrue(NetworkHelper.isIgnoredInterface("tun0"))
        assertTrue(NetworkHelper.isIgnoredInterface("tailscale0"))
        assertTrue(NetworkHelper.isIgnoredInterface("wg0"))
        assertTrue(NetworkHelper.isIgnoredInterface("tap0"))
        assertTrue(NetworkHelper.isIgnoredInterface("ppp0"))
        assertTrue(NetworkHelper.isIgnoredInterface("p2p0"))
        assertTrue(NetworkHelper.isIgnoredInterface("rmnet0"))
        assertTrue(NetworkHelper.isIgnoredInterface("ccmni0"))
        assertTrue(NetworkHelper.isIgnoredInterface("dummy0"))
        assertTrue(NetworkHelper.isIgnoredInterface("sit0"))
        assertTrue(NetworkHelper.isIgnoredInterface("ip6tnl0"))
        assertTrue(NetworkHelper.isIgnoredInterface("docker0"))
        assertTrue(NetworkHelper.isIgnoredInterface("veth123"))

        assertFalse(NetworkHelper.isIgnoredInterface("wlan0"))
        assertFalse(NetworkHelper.isIgnoredInterface("wlan1"))
        assertFalse(NetworkHelper.isIgnoredInterface("eth0"))
    }

    @Test
    fun testIsValidLocalIpv4() {
        val loopback = java.net.InetAddress.getByName("127.0.0.1")
        assertFalse(NetworkHelper.isValidLocalIpv4(loopback))

        val tailscaleCgnat = java.net.InetAddress.getByName("100.70.16.26")
        assertFalse(NetworkHelper.isValidLocalIpv4(tailscaleCgnat))

        val linkLocal = java.net.InetAddress.getByName("169.254.1.1")
        assertFalse(NetworkHelper.isValidLocalIpv4(linkLocal))

        val localWifi = java.net.InetAddress.getByName("192.168.1.100")
        assertTrue(NetworkHelper.isValidLocalIpv4(localWifi))

        val privateClassA = java.net.InetAddress.getByName("10.0.0.5")
        assertTrue(NetworkHelper.isValidLocalIpv4(privateClassA))
    }
}
