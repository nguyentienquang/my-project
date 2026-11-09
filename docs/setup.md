# WatchCam Setup & Deployment Guide

This guide covers setting up, installing, and configuring **WatchCam** on a dedicated Android tablet (such as the **IvyPad Smart 5** or any Android 11+ device) for 24/7 autonomous surveillance.

---

## 1. Hardware & System Prerequisites

- **Target Device:** IvyPad Smart 5 or any device running Android 11 (API level 30) or higher.
- **Architecture:** ARMv7 (32-bit), ARM64 (64-bit), or x86_64 emulator.
- **RAM:** 1 GB minimum (2 GB recommended).
- **Power:** Constant AC power supply (5V 2A micro-USB/Type-C adapter).
- **Network:** Connected to a 2.4 GHz or 5 GHz Wi-Fi local network.

---

## 2. Android OS Optimization for 24/7 Surveillance

To ensure the tablet operates continuously without entering deep sleep, closing the camera, or dropping Wi-Fi, apply the following system settings:

### 2.1 Enable Developer Options
1. Open **Cài đặt (Settings)** > **Giới thiệu về máy tính bảng (About tablet)**.
2. Tap **Số bản dựng (Build number)** 7 times until you see the toast: *"Bạn đã là nhà phát triển!" (You are now a developer!)*.
3. Return to **Cài đặt** > **Hệ thống (System)** > **Tùy chọn cho nhà phát triển (Developer options)**.
4. Enable **Gỡ lỗi USB (USB debugging)**.
5. Enable **Không tắt màn hình khi sạc (Stay awake)**: Keeps the screen awake or at minimal backlight while plugged into power.

### 2.2 Disable Battery Optimization for WatchCam
Android OS aggressive battery managers terminate background camera services if optimized:
1. Open **Cài đặt** > **Ứng dụng và thông báo (Apps & notifications)** > **Xem tất cả ứng dụng**.
2. Select **WatchCam**.
3. Tap **Pin (Battery)** > **Tối ưu hóa pin (Battery optimization)**.
4. Set WatchCam to **Không tối ưu hóa (Don't optimize / Unrestricted)**.

### 2.3 Wi-Fi Sleep Policy
Ensure Wi-Fi remains active indefinitely:
1. Open **Cài đặt** > **Mạng và Internet** > **Wi-Fi** > **Tùy chọn Wi-Fi**.
2. Disable any "Wi-Fi sleep when inactive" or "Power saving Wi-Fi" options.
3. **Static IP Recommendation:** Configure your Wi-Fi router to assign a static DHCP reservation (e.g. `192.168.1.150`) based on the tablet's MAC address so the URL never changes across reboots.

---

## 3. Installation Methods

### Method A: Direct Sideloading via ADB (Recommended)
1. Connect the tablet to your development computer via USB cable.
2. Confirm the connection in terminal:
   ```bash
   adb devices
   ```
   *(Ensure authorization prompt is accepted on the tablet screen).*
3. Install the APK:
   ```bash
   adb install -r app/build/outputs/apk/debug/app-debug.apk
   ```

### Method B: Standalone APK Transfer
1. Copy `app-debug.apk` to an SD card or USB flash drive, or transfer via local network drive.
2. Open the tablet's **Trình quản lý tệp (Files / File Manager)**.
3. Tap the APK to install. Allow installation from unknown sources if prompted.

---

## 4. Initial Launch & Operation

1. Tap the **WatchCam** icon on the tablet launcher.
2. Grant the system permissions:
   - **Máy ảnh (Camera):** Required for video frame acquisition.
   - **Thông báo (Notifications):** Required for the Foreground Service persistent status bar icon.
3. Tap the large green button **BẮT ĐẦU GIÁM SÁT** (Start Monitoring).
4. The status indicator will turn **Hoạt động (Active - Green)**.
5. Take note of:
   - **Địa chỉ truy cập (Dashboard URL):** e.g., `http://192.168.1.150:8080`
   - **Mã kết nối (6-digit Pairing Code):** e.g., `529148` (valid for 5 minutes).

---

## 5. Accessing the Web Surveillance Dashboard

1. On your desktop PC, laptop, or smartphone connected to the **same local Wi-Fi**, open any modern web browser (Google Chrome, Firefox, Safari, Microsoft Edge).
2. Enter the tablet's URL:
   ```
   http://192.168.1.150:8080
   ```
3. A pairing modal will appear in Vietnamese.
4. Enter the **6-digit pairing code** shown on the tablet screen.
5. Click **Xác nhận kết nối (Verify Pairing)**.
6. The dashboard will store the bearer session token in your browser's `localStorage` and immediately display:
   - Live surveillance video stream.
   - Real-time motion score gauge.
   - Notification chime on detected motion.
   - Event timeline with snapshots and deletion controls.
   - Remote camera switcher.

---

## 6. Physical Placement & Thermal Considerations

- **Mounting:** Use a generic tablet wall mount or stable stand positioned toward the entry point or monitoring zone.
- **Thermal Management:** Continuous camera sensor readout generates mild heat. Keep the tablet away from direct sunlight and enclosed unventilated cases.
- **Battery Health:** If deploying permanently, use a smart plug or low-wattage charger (5V 1.5A–2A) to prevent battery swelling over prolonged multi-month operation.
