# WatchCam Troubleshooting & Diagnostic Guide

This guide addresses common operational, network, camera, and performance issues encountered when running WatchCam.

---

## 1. Network & Connectivity Issues

### 1.1 Cannot access `http://<TABLET_IP>:8080` from browser
**Symptoms:** Browser displays `ERR_CONNECTION_TIMED_OUT` or `ERR_ADDRESS_UNREACHABLE`.

**Causes & Solutions:**
1. **Devices on Different Networks:**
   - Verify that your PC/phone and the Android tablet are connected to the exact same Wi-Fi SSID.
   - If your router has separate 2.4 GHz and 5 GHz networks or a "Guest Network", ensure both devices are on the primary network.
2. **Access Point (AP) Isolation:**
   - Many guest Wi-Fi networks enable **Client Isolation (AP Isolation)**, preventing wireless devices from communicating with one another.
   - Log into your Wi-Fi router admin page and disable **AP Isolation** for your home network.
3. **Windows Firewall / VPN:**
   - If accessing from a PC with an active corporate VPN or strict firewall, LAN traffic may be blocked. Temporarily disconnect the VPN or add an outbound rule.
4. **Android Wi-Fi Disconnected:**
   - Check the tablet screen. If the IP shows `Chưa kết nối Wi-Fi`, reconnect to Wi-Fi.

---

### 1.2 Tablet IP changes after router reboot
**Symptoms:** The dashboard URL stops working periodically because the tablet's IP changes (e.g. from `.150` to `.152`).

**Solution:**
- Open your Wi-Fi router's web console.
- Navigate to **DHCP Reservation / Static Leases**.
- Find the tablet by its MAC address (found in Android **Cài đặt > Giới thiệu về máy tính bảng > Địa chỉ MAC**).
- Assign a permanent static IP address (e.g. `192.168.1.150`).

---

### 1.3 Port 8080 already in use
**Symptoms:** The Android app displays `Lỗi khởi động máy chủ: Address already in use`.

**Solution:**
- Another app on the tablet is binding port 8080.
- Close conflicting applications or change the port configuration in settings.

---

## 2. Pairing & Authentication Issues

### 2.1 "Mã kết nối không hợp lệ hoặc đã hết hạn" (Invalid or Expired PIN)
**Symptoms:** Entering the 6-digit PIN into the web dashboard returns an invalid code error.

**Causes & Solutions:**
1. **5-Minute Timeout:**
   - Pairing codes expire automatically after 5 minutes for security.
   - Look at the Android tablet screen for the refreshed PIN, or tap the refresh icon on the tablet to generate a new PIN immediately.
2. **Rate Limit Lockout:**
   - Entering 5 incorrect PINs sequentially triggers a temporary security lockout (15 minutes).
   - Wait out the lockout period or restart the WatchCam surveillance service on the tablet.

---

### 2.2 Live Stream is black or broken image icon
**Symptoms:** The dashboard opens, but the video stream container shows a broken image icon.

**Causes & Solutions:**
1. **Session Token Missing or Expired:**
   - Click **Đăng xuất (Unpair / Logout)** in the upper right header and re-enter the active 6-digit pairing PIN.
2. **Surveillance Service Paused:**
   - Confirm that the Android tablet screen displays **Trạng thái: Hoạt động (Active)**. If stopped, tap **Bắt đầu giám sát**.

---

## 3. Camera Capture Issues

### 3.1 Camera initialization failure or "Camera In Use" error
**Symptoms:** The app reports `CameraUnavailableException` or fails to start preview.

**Causes & Solutions:**
1. **Another Application Holding Camera Lock:**
   - Android permits only one application to bind camera hardware at a time.
   - Force close all background camera apps (Instagram, Zalo, default Camera app).
2. **Missing Runtime Permissions:**
   - Go to **Cài đặt > Ứng dụng > WatchCam > Quyền (Permissions)** and ensure **Máy ảnh (Camera)** is set to **Cho phép (Allow)**.

---

### 3.2 Camera Switching Fails
**Symptoms:** Tapping "Đổi camera" (Switch Camera) does not switch to front/rear camera.

**Causes & Solutions:**
- Many low-cost tablets (including some IvyPad models) only possess a single camera sensor.
- If only one lens is physically present, WatchCam displays a toast notification: `Thiết bị chỉ có 1 camera khả dụng`.

---

## 4. Performance & Hardware Optimization

### 4.1 Tablet running hot or high battery drain
**Symptoms:** Device back cover feels warm during prolonged streaming.

**Causes & Solutions:**
1. **Lower Target Frame Rate:**
   - In the dashboard settings or configuration, decrease `target_fps` from 15 to **10** or **5 FPS**. For static surveillance, 5–10 FPS provides sufficient security monitoring while cutting CPU workload in half.
2. **Screen Brightness:**
   - Turn down tablet screen backlight to minimum. The screen does not need to be bright while acting as a camera sensor.
3. **Ensure Proper Air Circulation:**
   - Do not enclose the tablet in tight silicone cases during 24/7 plugged-in operation.

---

## 5. Motion Detection Tuning

### 5.1 Too many false alerts (High False Positives)
**Symptoms:** Frequent alerts triggered by tree shadows, small bugs, or light fluctuations.

**Solutions:**
1. Decrease **Độ nhạy chuyển động (Motion Sensitivity)** slider in the dashboard from default 50 down to **30–40**.
2. Increase the **Thời gian hồi chiêu (Cooldown)** to 10 seconds to limit consecutive alerts.
3. Ensure the camera lens is clean and not pointed directly at oscillating fans or reflective window surfaces.

### 5.2 Motion missed (False Negatives)
**Symptoms:** A person walks through the frame without triggering an alert.

**Solutions:**
1. Increase **Độ nhạy chuyển động** slider from 50 to **65–75**.
2. Verify that the monitored zone has sufficient ambient lighting (the sensor operates on luminance changes).
