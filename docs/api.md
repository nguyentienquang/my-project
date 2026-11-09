# WatchCam REST & WebSocket API Specification

The WatchCam embedded Axum server listens on port **8080** by default (e.g. `http://<TABLET_IP>:8080`).

---

## 1. Authentication

All media, event data, and remote control endpoints require authentication.

Authentication can be supplied via:
1. **HTTP Cookie:**
   ```http
   Cookie: watchcam_session=<TOKEN>
   ```
2. **HTTP Authorization Header:**
   ```http
   Authorization: Bearer <TOKEN>
   ```
3. **URL Query Parameter:** (ideal for `<img src="/stream.mjpeg?token=...">` and WebSocket connection)
   ```http
   ?token=<TOKEN>
   ```

Unauthorized requests return `401 Unauthorized`.

---

## 2. REST Endpoints

### 2.1 Web Dashboard Assets
- **Route:** `GET /`, `GET /index.html`, `GET /style.css`, `GET /app.js`
- **Auth Required:** No
- Serves the responsive surveillance web dashboard bundled inside the APK.

---

### 2.2 Authenticate / Pair Viewer
Establish an authenticated session using the 6-digit PIN shown on the tablet screen.

- **Route:** `POST /api/auth/pair`
- **Auth Required:** No (protected by rate limiting: max 5 failed attempts per 30 seconds)
- **Request Body:**
  ```json
  {
    "code": "839201"
  }
  ```
- **Response (200 OK):**
  ```json
  {
    "success": true,
    "token": "a1b2c3d4e5f6...",
    "message": null
  }
  ```
- **Error Responses:**
  - `401 Unauthorized`: `{"success": false, "token": null, "message": "Mã ghép nối không đúng!"}`

---

### 2.3 Logout / Revoke Session
Revoke the current authentication session token.

- **Route:** `POST /api/auth/logout`
- **Auth Required:** Yes
- **Response (200 OK):** `Đã đăng xuất` (clears cookie)

---

### 2.4 System Status & Metrics
Get operational status of the camera and surveillance engine.

- **Route:** `GET /api/status`
- **Auth Required:** Optional (returns `isAuthenticated: false` if unauthenticated)
- **Response (200 OK):**
  ```json
  {
    "cameraName": "Camera Sau",
    "isMonitoring": true,
    "actualFps": 10.2,
    "viewerCount": 1,
    "motionEnabled": true,
    "sensitivity": 70,
    "totalEvents": 14,
    "serverPort": 8080,
    "isAuthenticated": true
  }
  ```

---

### 2.5 Get Current Configuration
Retrieve current surveillance configuration.

- **Route:** `GET /api/config`
- **Auth Required:** Yes
- **Response (200 OK):**
  ```json
  {
    "motion_enabled": true,
    "sensitivity": 70,
    "pixel_diff_threshold": 26,
    "min_area_ratio": 0.0268,
    "min_consecutive_frames": 2,
    "cooldown_seconds": 5,
    "analysis_fps": 5,
    "stream_quality": 70,
    "target_stream_fps": 10,
    "server_port": 8080,
    "max_retention_days": 7,
    "max_storage_mb": 500,
    "camera_id": "back",
    "monitoring_region": null
  }
  ```

---

### 2.6 Update Configuration
Patch engine settings dynamically.

- **Route:** `PATCH /api/config`
- **Auth Required:** Yes
- **Request Body (all fields optional):**
  ```json
  {
    "motion_enabled": true,
    "sensitivity": 80,
    "stream_quality": 85,
    "switch_camera": true
  }
  ```
- **Response (200 OK):** Full updated configuration object.

---

### 2.7 Live Snapshot
Capture the latest processed camera frame immediately.

- **Route:** `GET /api/snapshot`
- **Auth Required:** Yes
- **Response (200 OK):**
  - `Content-Type: image/jpeg`
  - Binary JPEG image payload.

---

### 2.8 List Motion Events
Retrieve stored motion event records in reverse chronological order.

- **Route:** `GET /api/events`
- **Auth Required:** Yes
- **Response (200 OK):**
  ```json
  [
    {
      "id": "evt_1730999512000",
      "timestamp": 1730999512000,
      "motion_score": 0.35,
      "camera_id": "back",
      "snapshot_path": "/data/user/0/com.watchcam.android/files/snapshots/snap_evt_1730999512000.jpg",
      "event_type": "motion_detected",
      "region": null
    }
  ]
  ```

---

### 2.9 Get Event Details
- **Route:** `GET /api/events/:id`
- **Auth Required:** Yes
- **Response (200 OK):** JSON `MotionEvent` object.

---

### 2.10 Download Event Snapshot
Fetch the full JPEG snapshot captured at the moment motion triggered an alert.

- **Route:** `GET /api/events/:id/snapshot`
- **Auth Required:** Yes
- **Response (200 OK):** `Content-Type: image/jpeg`

---

### 2.11 Delete Single Event
- **Route:** `DELETE /api/events/:id`
- **Auth Required:** Yes
- **Response:** `204 No Content`

---

### 2.12 Clear All Events
- **Route:** `POST /api/events/clear`
- **Auth Required:** Yes
- **Response (200 OK):** `{"cleared": 14}`

---

### 2.13 Remote Camera Start / Stop
- **Route:** `POST /api/camera/start`
- **Route:** `POST /api/camera/stop`
- **Auth Required:** Yes
- **Response (200 OK):** Starts or stops surveillance pipeline.

---

## 3. Streaming Endpoint

### 3.1 Live MJPEG Video Stream
Standard multipart HTTP stream for browser `<img src="...">` rendering.

- **Route:** `GET /stream.mjpeg`
- **Auth Required:** Yes (via Cookie, `Authorization` header, or `?token=<TOKEN>`)
- **Response Headers:**
  ```http
  HTTP/1.1 200 OK
  Content-Type: multipart/x-mixed-replace; boundary=frame
  Cache-Control: no-cache, no-store, must-revalidate
  ```

---

## 4. WebSocket Notification Stream

### 4.1 Real-Time Alert Feed
Subscribes to live motion events and notifications.

- **Route:** `GET /ws/events`
- **Auth Required:** Yes (via Cookie or `?token=<TOKEN>`)
- **Protocol:** Standard WebSocket (`ws://` or `wss://`)

#### Outgoing Server Messages (JSON):
```json
{
  "type": "motion_detected",
  "eventId": "evt_1730999512000",
  "timestamp": 1730999512000,
  "motionScore": 0.42,
  "camera": "back",
  "snapshotAvailable": true
}
```
