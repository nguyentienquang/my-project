package com.watchcam.android

import com.watchcam.android.email.SmtpClient
import org.junit.Assert.*
import org.junit.Test

class SmtpClientTest {

    @Test
    fun testDefaultSmtpConfig() {
        val config = SmtpClient.SmtpConfig()
        assertEquals("smtp.gmail.com", config.host)
        assertEquals(587, config.port)
        assertEquals("", config.username)
        assertEquals("", config.password)
        assertEquals("", config.recipient)
        assertFalse(config.enabled)
    }

    @Test
    fun testDisabledConfigFailsFast() {
        val config = SmtpClient.SmtpConfig(
            host = "smtp.gmail.com",
            port = 587,
            username = "ban@gmail.com",
            password = "password",
            recipient = "nguoinhan@gmail.com",
            enabled = false
        )
        val result = SmtpClient.sendEmail(config, "Subject", "Body")
        assertTrue(result.isFailure)
        assertTrue(result.exceptionOrNull() is IllegalStateException)
    }

    @Test
    fun testBlankCredentialsFailsFast() {
        val config = SmtpClient.SmtpConfig(
            host = "smtp.gmail.com",
            port = 587,
            username = "",
            password = "",
            recipient = "",
            enabled = true
        )
        val result = SmtpClient.sendEmail(config, "Subject", "Body")
        assertTrue(result.isFailure)
        assertTrue(result.exceptionOrNull() is IllegalArgumentException)
    }

    @Test
    fun testWhitespaceOnlyCredentialsFailsFast() {
        val config = SmtpClient.SmtpConfig(
            host = "smtp.gmail.com",
            port = 587,
            username = "   ",
            password = "   ",
            recipient = "   ",
            enabled = true
        )
        val result = SmtpClient.sendEmail(config, "Subject", "Body")
        assertTrue(result.isFailure)
        assertTrue(result.exceptionOrNull() is IllegalArgumentException)
    }
}
