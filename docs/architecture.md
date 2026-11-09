# WatchCam System Architecture

## 1. Architectural Philosophy

WatchCam transforms resource-constrained Android devices (specifically modeled around the **IvyPad Smart 5**, Android 11 / API 30, ARMv7/ARM64 with 2 GB RAM) into a dedicated, self-contained IP surveillance camera.

To meet these strict hardware constraints, WatchCam adopts a **hybrid Android-Rust architecture**:
- **Android Platform Layer (Kotlin):** Handles Android lifecycle, runtime permissions, CameraX hardware abstraction, Foreground Service maintenance, and local UI.
- **Native Core Engine (Rust `cdylib`):** Handles frame throttling, image processing, grayscale JPEG compression, motion analysis, SQLite event indexing, bounded MJPEG streaming, WebSocket notifications, and HTTP serving.

By delegating CPU-intensive operations to native Rust, WatchCam eliminates Java Garbage Collection (GC) pauses, minimizes memory footprint to under 45 MB RSS, and prevents frame drops during continuous 24/7 surveillance.

---

## 2. High-Level System Architecture

```
+-------------------------------------------------------------------------------+
|                             Android Subsystem                                 |
|                                                                               |
|  +--------------------+     +-----------------------------------------------+ |
|  |    MainActivity    |     |              SurveillanceService              | |
|  | - Permission checks|     | - Android Foreground Service (camera type)    | |
|  | - Status display   |     | - CPU Partial WakeLock                        | |
|  | - Pairing code UI  |     | - Ongoing Notification with Stop Action       | |
|  +---------+----------+     +-----------------------+-----------------------+ |
|            |                                        |                         |
|            +-------------------+--------------------+                         |
|                                |                                              |
|                     +----------v-----------+                                  |
|                     |    CameraManager     |                                  |
|                     | - CameraX Lifecycle  |                                  |
|                     | - KEEP_ONLY_LATEST   |                                  |
|                     +----------+-----------+                                  |
|                                | Direct ByteBuffer (Y-Plane)                  |
+--------------------------------|----------------------------------------------+
                                 | JNI Call: processFrame()
+--------------------------------v----------------------------------------------+
|                    Rust Core Engine (libwatchcam_core.so)                     |
|                                                                               |
|  +-------------------------------------------------------------------------+  |
|  |                           WatchCamEngine                                |  |
|  | - Thread-safe Arc<EngineState>                                          |  |
|  | - Direct memory buffer reading (stride-aware copy)                      |  |
|  +---------------------+-----------------------------+---------------------+  |
|                        |                             |                        |
|                        v                             v                        |
|       +--------------------------------+   +--------------------------------+ |
|       |     Path A: Live Streaming     |   |    Path B: Motion Detection    | |
|       |                                |   |                                | |
|       | - Target FPS Rate Limiter      |   | - Low-res Downscaler (160x120) | |
|       | - Grayscale JPEG Encoder       |   | - Absolute Pixel Diff Engine   | |
|       | - Tokio watch::Sender (latest) |   | - Temporal Consecutive Filter  | |
|       | - Zero memory-leak ring buffer |   | - Dynamic Cooldown Suppressor  | |
|       +---------------+----------------+   +----------------+---------------+ |
|                       |                                     |                 |
|                       | Latest JPEG                         | Trigger Event   |
|                       v                                     v                 |
|       +--------------------------------+   +--------------------------------+ |
|       |      Axum HTTP Web Server      |   |        Storage Manager         | |
|       |                                |   |                                | |
|       | - Multipart MJPEG Streamer     |   | - SQLite Database (events.db)  | |
|       | - WebSocket Broadcaster (tokio)|   | - Disk JPEG Snapshot Storage   | |
|       | - REST API Endpoints           |   | - Auto-Retention Cleanup Task  | |
|       | - In-memory Web Assets Delivery|   |   (7-day max / 500 MB max)     | |
|       +---------------+----------------+   +--------------------------------+ |
+-----------------------|-------------------------------------------------------+
                        | HTTP Port 8080 (Wi-Fi / LAN)
                        v
+-------------------------------------------------------------------------------+
|                           Client Browser Dashboard                            |
|                                                                               |
|   - Real-time low-latency MJPEG canvas/img element                            |
|   - Reconnecting WebSocket subscriber for instant alerts                      |
|   - Web Audio API notification chime generator                                |
|   - Motion event snapshot gallery & timeline                                  |
|   - Remote camera switcher (Front / Rear)                                     |
+-------------------------------------------------------------------------------+
```

---

## 3. Detailed Subsystem Pipeline

### 3.1 Camera Capture & Zero-Copy JNI Ingestion

1. **CameraX Configuration:**
   - Strategy: `ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST`.
   - Resolution Selection: Fallback preference `1280×720` down to `640×480` for power-efficient sensor readout.
   - Format: `ImageFormat.YUV_420_888`.
2. **Buffer Extraction:**
   - In YUV_420_888, Plane 0 represents the Luminance (Y) channel.
   - CameraManager extracts the `ByteBuffer` directly from Plane 0 without allocating any intermediate Kotlin byte arrays.
   - Row stride and pixel stride are passed down across the JNI barrier alongside buffer dimensions.
3. **Safety Guarantee:**
   - `imageProxy.close()` is strictly executed inside a `finally` block in Kotlin, preventing camera pipeline buffer starvation.

### 3.2 Dual-Path Processing

#### Path A: Live Streaming Pipeline
1. **Rate Limiting:**
   - A millisecond-precision timestamp gate checks if `elapsed < 1000 / target_fps`.
   - Frames arriving above the target framerate (e.g., default 10 FPS) are dropped immediately with zero processing.
2. **Grayscale JPEG Encoding:**
   - Because surveillance does not strictly require full color, encoding directly from the 8-bit Y (luminance) plane provides:
     - **300% faster encoding** compared to YUV->RGB->JPEG conversion.
     - **60% smaller frame sizes** (typically 12–25 KB per 720p frame).
     - **Low CPU load** on IvyPad Smart 5 ARM cores.
   - Encoded using Rust's `image::codecs::jpeg::JpegEncoder` with quality setting 60–75%.
3. **Non-Blocking MJPEG Broadcast:**
   - Uses `tokio::sync::watch::Sender<Option<Arc<Vec<u8>>>>`.
   - The watch channel retains **strictly one frame**: the latest frame.
   - When a slow client lags behind on the local Wi-Fi, it simply drops intermediate frames. Memory consumption is **$O(1)$** regardless of client count or network jitter.

#### Path B: Motion Detection Pipeline
1. **Spatial Downscaling:**
   - The full-resolution Y-plane is downsampled via area-averaging / stride-skipping to a fixed grid of **160×120** pixels (19,200 bytes total).
2. **Differential Comparison:**
   - Pixel-by-pixel absolute difference is computed against the baseline frame:
     $$\Delta(x, y) = |I_t(x, y) - I_{t-1}(x, y)|$$
   - Pixels where $\Delta(x, y) > \text{pixel\_threshold}$ (default: 25 out of 255) are marked as changed.
3. **Motion Metric:**
   - The motion score is the percentage of changed pixels across the grid:
     $$\text{score} = \frac{\sum [\Delta(x, y) > \text{threshold}]}{160 \times 120} \times 100\%$$
4. **Temporal Confirmation Filter:**
   - Single-frame noise spikes (e.g. sensor gain fluctuations, sudden light flicker) are filtered out:
     - An alert triggers **only** when `score >= motion_threshold` for $N$ consecutive frames (default: $N = 2$).
5. **Dynamic Cooldown:**
   - Once triggered, an alert enters a cooldown period (default: 5.0 seconds).
   - During cooldown, snapshot creation and WebSocket sound triggers are suppressed, avoiding alert storms and disk spam.
6. **Snapshot Archival:**
   - When an alert triggers, the latest encoded JPEG frame is immediately persisted to the Android application's internal files directory (`/data/data/com.watchcam.android/files/snapshots/`).
   - Metadata is committed to SQLite `motion_events`.

---

## 4. Concurrency & Threading Model

WatchCam segregates operations into independent execution domains:

| Domain | Thread / Pool | Responsibility | Failure Containment |
|---|---|---|---|
| **Android UI** | Main Thread (Looper) | UI updates, button clicks, pairing display | Lightweight; never touches frames |
| **Camera Capture** | CameraX Background Executor | Image sensor readout, ImageProxy dispatch | Bounded by `KEEP_ONLY_LATEST` |
| **JNI Ingestion** | Native Bridge Thread | Copies Y-plane bytes to Rust heap | Reused buffer allocations |
| **Rust Async Server** | Tokio Multi-Thread Runtime (2 worker threads) | Axum routes, HTTP MJPEG, WebSockets | Non-blocking asynchronous I/O |
| **Storage / Retention** | Tokio `spawn_blocking` pool | SQLite queries, disk file I/O | Does not stall networking or stream |

---

## 5. Security & Authentication Architecture

1. **Transient Pairing Codes:**
   - A 6-digit random PIN is generated by `AuthManager` using a cryptographically secure RNG.
   - Code validity is capped at **5 minutes** (300 seconds).
2. **Lockout Protection:**
   - Failed pairing attempts are tracked per IP.
   - After 5 consecutive invalid attempts, an exponential lockout window (up to 15 minutes) is enforced.
3. **Session Tokens:**
   - Successful pairing generates a 256-bit hexadecimal bearer token.
   - Tokens can be provided via standard HTTP header `Authorization: Bearer <TOKEN>` or via query parameter `?token=<TOKEN>` (for HTML `<img>` tag and WebSocket connections).
4. **Local Network Boundary:**
   - The embedded server binds to `0.0.0.0:8080`.
   - The system is designed strictly for trusted LAN / Wi-Fi environments. It does not open external UPnP ports or establish reverse tunnels.
