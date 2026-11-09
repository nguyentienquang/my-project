package com.watchcam.android.email

import android.util.Base64
import android.util.Log
import java.io.BufferedReader
import java.io.File
import java.io.InputStreamReader
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.Socket
import javax.net.ssl.SSLSocket
import javax.net.ssl.SSLSocketFactory

object SmtpClient {
    private const val TAG = "WatchCam_SMTP"

    data class SmtpConfig(
        val host: String = "smtp.gmail.com",
        val port: Int = 587,
        val username: String = "",
        val password: String = "",
        val recipient: String = "",
        val enabled: Boolean = false
    )

    fun sendEmail(
        config: SmtpConfig,
        subject: String,
        bodyHtml: String,
        attachments: List<File> = emptyList()
    ): Result<Unit> {
        if (!config.enabled) {
            return Result.failure(IllegalStateException("Cảnh báo email đang bị tắt trong cấu hình"))
        }
        if (config.username.isBlank() || config.password.isBlank() || config.recipient.isBlank()) {
            return Result.failure(IllegalArgumentException("Thiếu thông tin tài khoản SMTP hoặc email người nhận"))
        }

        val cleanUsername = config.username.trim()
        val cleanPassword = config.password.replace(" ", "").trim()
        val cleanRecipient = config.recipient.trim()

        var socket: Socket? = null
        return try {
            val isSsl = config.port == 465
            Log.i(TAG, "Connecting to SMTP server ${config.host}:${config.port} (SSL=$isSsl)")

            val rawSocket = Socket()
            rawSocket.connect(InetSocketAddress(config.host, config.port), 15000)
            rawSocket.soTimeout = 15000

            var activeSocket: Socket = if (isSsl) {
                val sslFactory = SSLSocketFactory.getDefault() as SSLSocketFactory
                val ssl = sslFactory.createSocket(rawSocket, config.host, config.port, true) as SSLSocket
                ssl.startHandshake()
                ssl
            } else {
                rawSocket
            }
            socket = activeSocket

            var reader = BufferedReader(InputStreamReader(activeSocket.getInputStream(), Charsets.UTF_8))
            var writer: OutputStream = activeSocket.getOutputStream()

            fun readResponse(): String {
                val sb = StringBuilder()
                var line: String?
                while (reader.readLine().also { line = it } != null) {
                    val l = line!!
                    sb.append(l).append("\n")
                    if (l.length >= 4 && l[3] == ' ') break
                    if (l.length == 3) break
                }
                val resp = sb.toString().trim()
                Log.d(TAG, "<-- $resp")
                return resp
            }

            fun sendLine(cmd: String, redact: Boolean = false) {
                if (redact) {
                    Log.d(TAG, "--> [REDACTED]")
                } else {
                    Log.d(TAG, "--> $cmd")
                }
                writer.write((cmd + "\r\n").toByteArray(Charsets.UTF_8))
                writer.flush()
            }

            fun expectCode(prefix: String, context: String) {
                val resp = readResponse()
                if (!resp.startsWith(prefix)) {
                    throw IllegalStateException("SMTP error during $context: $resp")
                }
            }

            // 1. Initial banner
            expectCode("220", "Connect")

            // 2. EHLO
            sendLine("EHLO localhost")
            expectCode("250", "EHLO 1")

            // 3. STARTTLS if port 587
            if (!isSsl) {
                sendLine("STARTTLS")
                expectCode("220", "STARTTLS")

                val sslFactory = SSLSocketFactory.getDefault() as SSLSocketFactory
                val sslSocket = sslFactory.createSocket(activeSocket, config.host, config.port, true) as SSLSocket
                sslSocket.startHandshake()

                activeSocket = sslSocket
                socket = sslSocket
                reader = BufferedReader(InputStreamReader(activeSocket.getInputStream(), Charsets.UTF_8))
                writer = activeSocket.getOutputStream()

                sendLine("EHLO localhost")
                expectCode("250", "EHLO 2")
            }

            // 4. AUTH LOGIN
            sendLine("AUTH LOGIN")
            expectCode("334", "AUTH LOGIN")

            val userB64 = Base64.encodeToString(cleanUsername.toByteArray(Charsets.UTF_8), Base64.NO_WRAP)
            sendLine(userB64)
            expectCode("334", "Username")

            val passB64 = Base64.encodeToString(cleanPassword.toByteArray(Charsets.UTF_8), Base64.NO_WRAP)
            sendLine(passB64, redact = true)
            expectCode("235", "Authentication")

            // 5. Envelope
            sendLine("MAIL FROM:<$cleanUsername>")
            expectCode("250", "MAIL FROM")

            sendLine("RCPT TO:<$cleanRecipient>")
            expectCode("250", "RCPT TO")

            // 6. DATA
            sendLine("DATA")
            expectCode("354", "DATA")

            // 7. MIME Headers
            val boundary = "==_WatchCam_Part_${System.currentTimeMillis()}_=="
            val mimeHeaders = StringBuilder().apply {
                append("From: WatchCam Surveillance <$cleanUsername>\r\n")
                append("To: <$cleanRecipient>\r\n")
                append("Subject: =?UTF-8?B?${Base64.encodeToString(subject.toByteArray(Charsets.UTF_8), Base64.NO_WRAP)}?=\r\n")
                append("MIME-Version: 1.0\r\n")
                append("Content-Type: multipart/mixed; boundary=\"$boundary\"\r\n")
                append("\r\n")
            }
            writer.write(mimeHeaders.toString().toByteArray(Charsets.UTF_8))

            // Body Part
            val bodyPart = StringBuilder().apply {
                append("--$boundary\r\n")
                append("Content-Type: text/html; charset=UTF-8\r\n")
                append("Content-Transfer-Encoding: base64\r\n\r\n")
                append(Base64.encodeToString(bodyHtml.toByteArray(Charsets.UTF_8), Base64.NO_WRAP))
                append("\r\n")
            }
            writer.write(bodyPart.toString().toByteArray(Charsets.UTF_8))

            // Attachments
            for (file in attachments) {
                if (!file.exists() || file.length() == 0L) continue

                val mimeType = when {
                    file.name.endsWith(".mp4", ignoreCase = true) -> "video/mp4"
                    file.name.endsWith(".jpg", ignoreCase = true) || file.name.endsWith(".jpeg", ignoreCase = true) -> "image/jpeg"
                    else -> "application/octet-stream"
                }

                val attachHeader = StringBuilder().apply {
                    append("--$boundary\r\n")
                    append("Content-Type: $mimeType; name=\"${file.name}\"\r\n")
                    append("Content-Disposition: attachment; filename=\"${file.name}\"\r\n")
                    append("Content-Transfer-Encoding: base64\r\n\r\n")
                }
                writer.write(attachHeader.toString().toByteArray(Charsets.UTF_8))

                file.inputStream().use { fis ->
                    val buffer = ByteArray(45 * 1024)
                    while (true) {
                        var bytesRead = 0
                        while (bytesRead < buffer.size) {
                            val r = fis.read(buffer, bytesRead, buffer.size - bytesRead)
                            if (r == -1) break
                            bytesRead += r
                        }
                        if (bytesRead == 0) break

                        val chunk = if (bytesRead == buffer.size) buffer else buffer.copyOf(bytesRead)
                        val b64 = Base64.encodeToString(chunk, Base64.NO_WRAP)
                        writer.write(b64.toByteArray(Charsets.US_ASCII))
                        writer.write("\r\n".toByteArray(Charsets.US_ASCII))

                        if (bytesRead < buffer.size) break // Reached EOF
                    }
                }
                writer.write("\r\n".toByteArray(Charsets.US_ASCII))
            }

            // End Boundary & End of DATA
            writer.write(("--$boundary--\r\n").toByteArray(Charsets.UTF_8))
            writer.write(".\r\n".toByteArray(Charsets.UTF_8))
            writer.flush()
            expectCode("250", "End of message")

            // 8. QUIT
            sendLine("QUIT")
            try { socket.close() } catch (_: Exception) {}

            Log.i(TAG, "Email alert sent successfully to ${config.recipient}")
            Result.success(Unit)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to send email alert", e)
            try { socket?.close() } catch (_: Exception) {}
            Result.failure(e)
        }
    }
}
