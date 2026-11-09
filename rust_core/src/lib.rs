pub mod auth;
pub mod config;
pub mod jni_api;
pub mod motion;
pub mod server;
pub mod storage;
pub mod stream;
pub mod tls;
pub mod web_assets;

use auth::AuthManager;
use config::SurveillanceConfig;
use motion::{MotionDetector, MotionEvent};
use parking_lot::{Mutex, RwLock};
use server::{create_router, AppState};
use storage::EventDatabase;
use stream::FrameBroadcaster;

use image::codecs::jpeg::JpegEncoder;
use image::{ColorType, ImageEncoder};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;
use tokio::sync::broadcast;

pub struct WatchCamEngine {
    pub app_state: AppState,
    pub motion_detector: Arc<Mutex<MotionDetector>>,
    pub rgb_scratch_buffer: Arc<Mutex<Vec<u8>>>,
    pub storage_dir: PathBuf,
    tokio_runtime: Option<Runtime>,
    server_abort_handles: Vec<tokio::task::AbortHandle>,
}

static ENGINE: Mutex<Option<WatchCamEngine>> = Mutex::new(None);
static LAST_DB_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

impl WatchCamEngine {
    pub fn initialize<P: AsRef<Path>>(storage_path: P, db_path: P, port: u16) -> Result<(), String> {
        // Stop any running engine first to guarantee port and runtime cleanup
        Self::stop();

        *LAST_DB_PATH.lock() = Some(db_path.as_ref().to_path_buf());

        let storage_dir = storage_path.as_ref().to_path_buf();
        let _ = fs::create_dir_all(&storage_dir);

        let db = EventDatabase::new(db_path, &storage_dir)
            .map_err(|e| format!("Lỗi khởi tạo SQLite: {}", e))?;
        let db_arc = Arc::new(db);

        let mut config = SurveillanceConfig::default();
        config.server_port = port;
        config.https_port = if port == 8080 { 8443 } else { port.wrapping_add(363) };

        let auth = AuthManager::with_db(db_arc.clone());
        let broadcaster = FrameBroadcaster::new();
        let audio_broadcaster = crate::stream::AudioBroadcaster::new();
        let (ws_sender, _) = broadcast::channel(128);

        let app_state = AppState {
            config: Arc::new(RwLock::new(config.clone())),
            auth,
            broadcaster,
            audio_broadcaster,
            db: db_arc,
            ws_sender,
            is_monitoring: Arc::new(AtomicBool::new(true)),
            switch_camera_requested: Arc::new(AtomicBool::new(false)),
            resolution_change_requested: Arc::new(AtomicU8::new(0)),
            test_email_requested: Arc::new(AtomicBool::new(false)),
            device_status: Arc::new(RwLock::new(None)),
        };

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| format!("Lỗi khởi tạo Tokio: {}", e))?;

        let router = create_router(app_state.clone());
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));

        let http_router = router.clone();
        let http_handle = rt.spawn(async move {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    let _ = axum::serve(listener, http_router).await;
                }
                Err(e) => {
                    eprintln!("Lỗi bind HTTP server: {:?}", e);
                }
            }
        }).abort_handle();

        let https_port = config.https_port;
        let https_router = router;
        let https_handle = rt.spawn(async move {
            match crate::tls::create_rustls_config().await {
                Ok(tls_config) => {
                    let https_addr = std::net::SocketAddr::from(([0, 0, 0, 0], https_port));
                    match std::net::TcpListener::bind(https_addr) {
                        Ok(std_listener) => {
                            if let Err(e) = std_listener.set_nonblocking(true) {
                                eprintln!("Lỗi set nonblocking HTTPS listener: {:?}", e);
                            }
                            let _ = axum_server::from_tcp_rustls(std_listener, tls_config)
                                .serve(https_router.into_make_service())
                                .await;
                        }
                        Err(e) => {
                            eprintln!("Lỗi bind HTTPS server trên cổng {}: {:?}", https_port, e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Lỗi khởi tạo TLS config cho HTTPS server: {:?}", e);
                }
            }
        }).abort_handle();

        let engine = WatchCamEngine {
            app_state,
            motion_detector: Arc::new(Mutex::new(MotionDetector::new())),
            rgb_scratch_buffer: Arc::new(Mutex::new(Vec::with_capacity(1280 * 720 * 3))),
            storage_dir,
            tokio_runtime: Some(rt),
            server_abort_handles: vec![http_handle, https_handle],
        };

        *ENGINE.lock() = Some(engine);
        Ok(())
    }

    pub fn stop() {
        let mut lock = ENGINE.lock();
        if let Some(mut engine) = lock.take() {
            for handle in engine.server_abort_handles.drain(..) {
                handle.abort();
            }
            if let Some(rt) = engine.tokio_runtime.take() {
                rt.shutdown_timeout(std::time::Duration::from_millis(500));
            }
        }
    }

    /// Process a raw YUV_420_888 frame from CameraX.
    /// Returns: (motion_alert_triggered, motion_score, should_switch_camera, is_monitoring, res_change_code, test_email_requested, entity_code, confidence, bbox_x, bbox_y, bbox_w, bbox_h)
    pub fn process_camera_frame_yuv(
        y_plane: &[u8],
        u_plane: &[u8],
        v_plane: &[u8],
        width: usize,
        height: usize,
        y_row_stride: usize,
        u_row_stride: usize,
        v_row_stride: usize,
        u_pixel_stride: usize,
        v_pixel_stride: usize,
        rotation_degrees: i32,
    ) -> (bool, f32, bool, bool, f32, bool, f32, f32, f32, f32, f32, f32) {
        let engine_guard = ENGINE.lock();
        let engine = match engine_guard.as_ref() {
            Some(e) => e,
            None => return (false, 0.0, false, false, 0.0, false, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        };

        let is_monitoring = engine.app_state.is_monitoring.load(Ordering::Relaxed);
        let should_switch_cam = engine.app_state.switch_camera_requested.swap(false, Ordering::Relaxed);
        let res_change = engine.app_state.resolution_change_requested.swap(0, Ordering::Relaxed) as f32;
        let test_email_req = engine.app_state.test_email_requested.swap(false, Ordering::Relaxed);

        if !is_monitoring {
            return (false, 0.0, should_switch_cam, false, res_change, test_email_req, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        }

        let now = Instant::now();
        let now_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let config = engine.app_state.config.read().clone();

        // 1. Motion Analysis: Path B (Zero-allocation direct downsampling & rotating on Y-plane)
        let (alert_triggered, motion_score, entity_type, confidence, bounding_box) = if config.motion_enabled {
            let motion_res = {
                let mut detector = engine.motion_detector.lock();
                detector.process_raw_frame(
                    y_plane,
                    width,
                    height,
                    y_row_stride,
                    rotation_degrees,
                    &config,
                    now,
                )
            };
            (
                motion_res.is_alert_triggered,
                motion_res.motion_score,
                motion_res.entity_type,
                motion_res.confidence,
                motion_res.bounding_box,
            )
        } else {
            // When motion detection is disabled: completely disable motion analysis and reset detector state
            let mut detector = engine.motion_detector.lock();
            detector.reset();
            (false, 0.0, "motion".to_string(), 0.0, None)
        };

        // Check if we need to convert to RGB:
        // We convert to RGB if:
        // a) An alert triggered (to save full-color event snapshot)
        // b) Broadcaster wants to encode a live frame (viewer connected or 1 FPS keep-alive)
        let viewer_count = engine.app_state.broadcaster.get_viewer_count();
        let stream_fps = if viewer_count > 0 {
            config.target_stream_fps
        } else {
            1
        };

        let need_stream_frame = engine.app_state.broadcaster.should_encode_frame(stream_fps, now);

        if alert_triggered || need_stream_frame {
            // Convert to true color RGB8 with upright orientation into reusable buffer
            let mut rgb_guard = engine.rgb_scratch_buffer.lock();
            let (oriented_w, oriented_h) = yuv420_to_rgb_into(
                y_plane,
                u_plane,
                v_plane,
                width,
                height,
                y_row_stride,
                u_row_stride,
                v_row_stride,
                u_pixel_stride,
                v_pixel_stride,
                rotation_degrees,
                &mut rgb_guard,
            );

            if !rgb_guard.is_empty() && oriented_w > 0 && oriented_h > 0 {
                // Encode color JPEG
                if let Ok(jpeg_bytes) = encode_rgb_to_jpeg(&rgb_guard, oriented_w, oriented_h, config.stream_quality) {
                    // Save alert snapshot if triggered
                    if alert_triggered && config.motion_enabled {
                        let event_id = format!("evt_{}", now_millis);
                        let snap_filename = format!("snap_{}.jpg", event_id);
                        let snap_path = engine.storage_dir.join(&snap_filename);

                        let _ = fs::write(&snap_path, &jpeg_bytes);

                        let event_type_str = match entity_type.as_str() {
                            "person" => "person_detected".to_string(),
                            "object" => "object_detected".to_string(),
                            _ => "motion_detected".to_string(),
                        };

                        let event = MotionEvent {
                            id: event_id.clone(),
                            timestamp: now_millis,
                            motion_score,
                            camera_id: config.camera_id.clone(),
                            snapshot_path: snap_path.to_string_lossy().to_string(),
                            event_type: event_type_str.clone(),
                            region: bounding_box.or(config.monitoring_region),
                            video_path: None,
                            has_video: false,
                            entity_type: entity_type.clone(),
                            confidence,
                        };

                        let _ = engine.app_state.db.insert_event(&event);

                        let ws_payload = serde_json::json!({
                            "type": event_type_str,
                            "eventId": event_id,
                            "timestamp": now_millis,
                            "motionScore": motion_score,
                            "camera": config.camera_id,
                            "snapshotAvailable": true,
                            "entityType": entity_type,
                            "confidence": confidence,
                            "boundingBox": bounding_box,
                            "region": bounding_box.or(config.monitoring_region),
                        });
                        let _ = engine.app_state.ws_sender.send(ws_payload.to_string());

                        let _ = engine.app_state.db.enforce_retention(config.max_retention_days, config.max_storage_mb);
                    }

                    // Broadcast live frame if due
                    if need_stream_frame {
                        engine.app_state.broadcaster.broadcast_frame(
                            jpeg_bytes,
                            oriented_w as u32,
                            oriented_h as u32,
                            now_millis,
                        );

                        // If viewers are watching the live stream and motion or bounding box is active,
                        // broadcast live detection overlay updates (without triggering alert cooldown/sound)
                        let is_active_detection = bounding_box.is_some() || motion_score >= config.min_area_ratio;
                        let should_send_clear = {
                            let mut det = engine.motion_detector.lock();
                            det.update_active_detection_state(is_active_detection)
                        };

                        if viewer_count > 0 && !alert_triggered {
                            if is_active_detection {
                                let ws_live_payload = serde_json::json!({
                                    "type": "detection_update",
                                    "timestamp": now_millis,
                                    "motionScore": motion_score,
                                    "camera": config.camera_id,
                                    "entityType": entity_type,
                                    "confidence": confidence,
                                    "boundingBox": bounding_box,
                                    "region": bounding_box.or(config.monitoring_region),
                                });
                                let _ = engine.app_state.ws_sender.send(ws_live_payload.to_string());
                            } else if should_send_clear {
                                let ws_clear_payload = serde_json::json!({
                                    "type": "detection_update",
                                    "timestamp": now_millis,
                                    "motionScore": 0.0,
                                    "camera": config.camera_id,
                                    "entityType": "motion",
                                    "confidence": 0.0,
                                    "boundingBox": None::<[f32; 4]>,
                                    "region": None::<[f32; 4]>,
                                });
                                let _ = engine.app_state.ws_sender.send(ws_clear_payload.to_string());
                            }
                        }
                    }
                }
            }
        }

        let entity_code = match entity_type.as_str() {
            "person" => 1.0f32,
            "object" => 2.0f32,
            "motion" => 3.0f32,
            _ => 0.0f32,
        };
        let (bx, by, bw, bh) = if let Some([x, y, w, h]) = bounding_box {
            (x, y, w, h)
        } else {
            (0.0f32, 0.0f32, 0.0f32, 0.0f32)
        };

        (
            alert_triggered,
            motion_score,
            should_switch_cam,
            true,
            res_change,
            test_email_req,
            entity_code,
            confidence,
            bx,
            by,
            bw,
            bh,
        )
    }

    /// Process a raw Y-plane frame from CameraX (backward compatibility wrapper).
    /// Returns: (motion_alert_triggered, motion_score, should_switch_camera, is_monitoring)
    pub fn process_camera_frame(
        y_plane: &[u8],
        width: usize,
        height: usize,
        row_stride: usize,
        rotation_degrees: i32,
    ) -> (bool, f32, bool, bool) {
        let (alert, score, switch_cam, mon, _, _, _, _, _, _, _, _) = Self::process_camera_frame_yuv(
            y_plane,
            &[],
            &[],
            width,
            height,
            row_stride,
            0,
            0,
            0,
            0,
            rotation_degrees,
        );
        (alert, score, switch_cam, mon)
    }

    pub fn update_config(motion_enabled: bool, sensitivity: u8, stream_quality: u8) {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            let mut cfg = engine.app_state.config.write();
            cfg.motion_enabled = motion_enabled;
            cfg.apply_sensitivity(sensitivity);
            if stream_quality > 0 {
                cfg.stream_quality = stream_quality.clamp(10, 100);
            }
            let ws_msg = serde_json::json!({
                "type": "config_updated",
                "motionEnabled": cfg.motion_enabled,
                "sensitivity": cfg.sensitivity,
                "streamQuality": cfg.stream_quality,
                "cooldownSeconds": cfg.cooldown_seconds,
                "resolution": cfg.resolution,
                "emailAlertsEnabled": cfg.email_alerts_enabled,
            });
            let _ = engine.app_state.ws_sender.send(ws_msg.to_string());
        }
    }

    pub fn update_email_config(
        enabled: bool,
        host: &str,
        port: u16,
        username: &str,
        password: &str,
        recipient: &str,
    ) {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            let mut cfg = engine.app_state.config.write();
            cfg.email_alerts_enabled = enabled;
            let host_trimmed = host.trim();
            if !host_trimmed.is_empty() {
                cfg.smtp_host = host_trimmed.to_string();
            }
            if port > 0 {
                cfg.smtp_port = port;
            }
            cfg.smtp_username = username.trim().to_string();
            if !password.is_empty() {
                cfg.smtp_password = password.trim().to_string();
            }
            cfg.smtp_recipient = recipient.trim().to_string();

            let ws_msg = serde_json::json!({
                "type": "config_updated",
                "motionEnabled": cfg.motion_enabled,
                "sensitivity": cfg.sensitivity,
                "streamQuality": cfg.stream_quality,
                "targetStreamFps": cfg.target_stream_fps,
                "cooldownSeconds": cfg.cooldown_seconds,
                "resolution": cfg.resolution,
                "emailAlertsEnabled": cfg.email_alerts_enabled,
                "smtpHost": cfg.smtp_host,
                "smtpPort": cfg.smtp_port,
                "smtpUsername": cfg.smtp_username,
                "smtpRecipient": cfg.smtp_recipient,
            });
            let _ = engine.app_state.ws_sender.send(ws_msg.to_string());
        }
    }

    pub fn regenerate_pairing_code() -> String {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            engine.app_state.auth.regenerate_pairing_code()
        } else {
            "------".to_string()
        }
    }

    pub fn revoke_all_sessions() {
        Self::revoke_all_sessions_with_db(None);
    }

    pub fn revoke_all_sessions_with_db(custom_db_path: Option<&Path>) {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            engine.app_state.auth.revoke_all_sessions();
            let revoke_msg = serde_json::json!({
                "type": "sessions_revoked"
            });
            let _ = engine.app_state.ws_sender.send(revoke_msg.to_string());
        }

        let db_to_clean = custom_db_path
            .map(|p| p.to_path_buf())
            .or_else(|| LAST_DB_PATH.lock().clone());

        if let Some(db_path) = db_to_clean {
            if db_path.exists() {
                if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                    let _ = conn.execute("DELETE FROM paired_devices", []);
                }
            }
        }
    }

    pub fn set_camera_id(is_front: bool) {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            let mut cfg = engine.app_state.config.write();
            cfg.camera_id = if is_front { "front" } else { "back" }.to_string();
            engine.motion_detector.lock().reset();
        }
    }

    pub fn set_monitoring(enabled: bool) {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            engine.app_state.is_monitoring.store(enabled, Ordering::Relaxed);
        }
    }

    pub fn is_monitoring() -> bool {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            engine.app_state.is_monitoring.load(Ordering::Relaxed)
        } else {
            false
        }
    }

    pub fn get_pairing_code() -> String {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            engine.app_state.auth.get_pairing_code()
        } else {
            "------".to_string()
        }
    }

    pub fn get_status_json() -> String {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            let cfg = engine.app_state.config.read().clone();
            let total_events = engine.app_state.db.count_events().unwrap_or(0);
            let pairing_code = engine.app_state.auth.get_pairing_code();
            let device_status = engine.app_state.device_status.read().clone();

            serde_json::json!({
                "cameraName": if cfg.camera_id == "front" { "Camera Trước" } else { "Camera Sau" },
                "isMonitoring": engine.app_state.is_monitoring.load(Ordering::Relaxed),
                "actualFps": engine.app_state.broadcaster.get_actual_fps(),
                "viewerCount": engine.app_state.broadcaster.get_viewer_count(),
                "motionEnabled": cfg.motion_enabled,
                "sensitivity": cfg.sensitivity,
                "streamQuality": cfg.stream_quality,
                "cooldownSeconds": cfg.cooldown_seconds,
                "resolution": cfg.resolution,
                "emailAlertsEnabled": cfg.email_alerts_enabled,
                "smtpHost": cfg.smtp_host,
                "smtpPort": cfg.smtp_port,
                "smtpUsername": cfg.smtp_username,
                "smtpPassword": cfg.smtp_password,
                "smtpRecipient": cfg.smtp_recipient,
                "totalEvents": total_events,
                "serverPort": cfg.server_port,
                "httpsPort": cfg.https_port,
                "pairingCode": pairing_code,
                "device": device_status,
                "audioLevel": engine.app_state.audio_broadcaster.get_audio_level(),
                "audioViewerCount": engine.app_state.audio_broadcaster.get_viewer_count(),
            }).to_string()
        } else {
            "{}".to_string()
        }
    }

    pub fn update_device_status(json_str: &str) -> bool {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                *engine.app_state.device_status.write() = Some(val.clone());
                let ws_msg = serde_json::json!({
                    "type": "device_status",
                    "device": val,
                });
                let _ = engine.app_state.ws_sender.send(ws_msg.to_string());
                return true;
            }
        }
        false
    }

    pub fn get_device_status_json() -> String {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            if let Some(device) = engine.app_state.device_status.read().as_ref() {
                return device.to_string();
            }
        }
        "{}".to_string()
    }

    pub fn get_latest_event_json() -> String {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            if let Ok(Some(ev)) = engine.app_state.db.get_latest_event() {
                return serde_json::to_string(&ev).unwrap_or_default();
            }
        }
        String::new()
    }

    pub fn notify_video_recorded(event_id: &str, video_path: &str) -> bool {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            let _ = engine.app_state.db.update_event_video_path(event_id, video_path);
            let ws_msg = serde_json::json!({
                "type": "video_recorded",
                "eventId": event_id,
                "videoUrl": format!("/api/events/{}/video", event_id),
                "hasVideo": true
            });
            let _ = engine.app_state.ws_sender.send(ws_msg.to_string());
            true
        } else {
            false
        }
    }

    pub fn push_audio_frame(data: &[u8], level: u8) {
        let lock = ENGINE.lock();
        if let Some(engine) = lock.as_ref() {
            engine.app_state.audio_broadcaster.broadcast_audio(data, level);
        }
    }
}

/// Packs a strided Y plane buffer and rotates it according to rotation_degrees (0, 90, 180, 270).
/// Returns: (packed_buffer, new_width, new_height)
pub fn pack_and_rotate_y_plane(
    src: &[u8],
    width: usize,
    height: usize,
    row_stride: usize,
    rotation_degrees: i32,
) -> (Vec<u8>, usize, usize) {
    if src.is_empty() || width == 0 || height == 0 {
        return (Vec::new(), 0, 0);
    }

    let norm_rotation = ((rotation_degrees % 360) + 360) % 360;

    match norm_rotation {
        90 => {
            let new_width = height;
            let new_height = width;
            let mut dst = vec![0u8; new_width * new_height];
            for y in 0..height {
                let src_row = y * row_stride;
                for x in 0..width {
                    let src_idx = src_row + x;
                    if src_idx < src.len() {
                        let dst_idx = x * new_width + (height - 1 - y);
                        dst[dst_idx] = src[src_idx];
                    }
                }
            }
            (dst, new_width, new_height)
        }
        180 => {
            let mut dst = vec![0u8; width * height];
            for y in 0..height {
                let src_row = y * row_stride;
                let dst_row = (height - 1 - y) * width;
                for x in 0..width {
                    let src_idx = src_row + x;
                    if src_idx < src.len() {
                        let dst_idx = dst_row + (width - 1 - x);
                        dst[dst_idx] = src[src_idx];
                    }
                }
            }
            (dst, width, height)
        }
        270 => {
            let new_width = height;
            let new_height = width;
            let mut dst = vec![0u8; new_width * new_height];
            for y in 0..height {
                let src_row = y * row_stride;
                for x in 0..width {
                    let src_idx = src_row + x;
                    if src_idx < src.len() {
                        let dst_idx = (width - 1 - x) * new_width + y;
                        dst[dst_idx] = src[src_idx];
                    }
                }
            }
            (dst, new_width, new_height)
        }
        _ => {
            // 0 degrees or standard
            let mut dst = vec![0u8; width * height];
            if row_stride == width && src.len() >= width * height {
                dst.copy_from_slice(&src[..width * height]);
            } else {
                for y in 0..height {
                    let src_row = y * row_stride;
                    let dst_row = y * width;
                    let src_end = src_row + width;
                    if src_end <= src.len() {
                        dst[dst_row..dst_row + width].copy_from_slice(&src[src_row..src_end]);
                    } else if src_row < src.len() {
                        let available = src.len() - src_row;
                        let len = available.min(width);
                        dst[dst_row..dst_row + len].copy_from_slice(&src[src_row..src_row + len]);
                    }
                }
            }
            (dst, width, height)
        }
    }
}

// Precomputed BT.601 integer lookup tables for high-performance zero-divergence YUV to RGB conversion
const fn make_r_diff() -> [i32; 256] {
    let mut arr = [0i32; 256];
    let mut i = 0;
    while i < 256 {
        let v = i as i32 - 128;
        // R = Y + 1.40200 * (Cr - 128) -> (1436 * v + 512) >> 10
        arr[i] = (1436 * v + 512) >> 10;
        i += 1;
    }
    arr
}

const fn make_b_diff() -> [i32; 256] {
    let mut arr = [0i32; 256];
    let mut i = 0;
    while i < 256 {
        let u = i as i32 - 128;
        // B = Y + 1.77200 * (Cb - 128) -> (1815 * u + 512) >> 10
        arr[i] = (1815 * u + 512) >> 10;
        i += 1;
    }
    arr
}

const fn make_g_diff_u() -> [i32; 256] {
    let mut arr = [0i32; 256];
    let mut i = 0;
    while i < 256 {
        let u = i as i32 - 128;
        // G_u = 0.34414 * (Cb - 128) -> (352 * u + 512) >> 10
        arr[i] = (352 * u + 512) >> 10;
        i += 1;
    }
    arr
}

const fn make_g_diff_v() -> [i32; 256] {
    let mut arr = [0i32; 256];
    let mut i = 0;
    while i < 256 {
        let v = i as i32 - 128;
        // G_v = 0.71414 * (Cr - 128) -> (731 * v + 512) >> 10
        arr[i] = (731 * v + 512) >> 10;
        i += 1;
    }
    arr
}

pub static R_DIFF: [i32; 256] = make_r_diff();
pub static B_DIFF: [i32; 256] = make_b_diff();
pub static G_DIFF_U: [i32; 256] = make_g_diff_u();
pub static G_DIFF_V: [i32; 256] = make_g_diff_v();

#[inline(always)]
pub fn yuv_to_rgb_pixel(y: u8, u: u8, v: u8) -> (u8, u8, u8) {
    let y_i = y as i32;
    let r = (y_i + R_DIFF[v as usize]).clamp(0, 255) as u8;
    let g = (y_i - G_DIFF_U[u as usize] - G_DIFF_V[v as usize]).clamp(0, 255) as u8;
    let b = (y_i + B_DIFF[u as usize]).clamp(0, 255) as u8;
    (r, g, b)
}

/// Converts YUV_420_888 planes (with arbitrary row strides and pixel strides) into a pre-allocated upright RGB8 buffer.
/// Rotates according to rotation_degrees (0, 90, 180, 270) with zero heap allocations on steady-state.
/// Returns: (oriented_width, oriented_height)
pub fn yuv420_to_rgb_into(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    width: usize,
    height: usize,
    y_row_stride: usize,
    u_row_stride: usize,
    v_row_stride: usize,
    u_pixel_stride: usize,
    v_pixel_stride: usize,
    rotation_degrees: i32,
    dst: &mut Vec<u8>,
) -> (usize, usize) {
    if y_plane.is_empty() || width == 0 || height == 0 {
        dst.clear();
        return (0, 0);
    }

    let norm_rot = ((rotation_degrees % 360) + 360) % 360;
    let (out_w, out_h) = match norm_rot {
        90 | 270 => (height, width),
        _ => (width, height),
    };

    let target_len = out_w * out_h * 3;
    if dst.len() != target_len {
        dst.resize(target_len, 0);
    }

    let has_u = !u_plane.is_empty() && u_row_stride > 0;
    let has_v = !v_plane.is_empty() && v_row_stride > 0;
    let u_px_stride = if u_pixel_stride > 0 { u_pixel_stride } else { 1 };
    let v_px_stride = if v_pixel_stride > 0 { v_pixel_stride } else { 1 };

    let y_len = y_plane.len();
    let u_len = u_plane.len();
    let v_len = v_plane.len();

    match norm_rot {
        90 => {
            // Clockwise 90 deg: out_w = height, out_h = width
            let mut dst_idx = 0;
            for out_y in 0..out_h {
                let src_x = out_y;
                let uv_x = src_x >> 1;
                let uv_u_x_offset = uv_x * u_px_stride;
                let uv_v_x_offset = uv_x * v_px_stride;

                for out_x in 0..out_w {
                    let src_y = height.saturating_sub(1).saturating_sub(out_x);
                    let y_idx = src_y * y_row_stride + src_x;
                    let y_val = if y_idx < y_len { y_plane[y_idx] } else { 0 };

                    let uv_y = src_y >> 1;
                    let u_idx = uv_y * u_row_stride + uv_u_x_offset;
                    let u_val = if has_u && u_idx < u_len { u_plane[u_idx] } else { 128 };

                    let v_idx = uv_y * v_row_stride + uv_v_x_offset;
                    let v_val = if has_v && v_idx < v_len { v_plane[v_idx] } else { 128 };

                    let (r, g, b) = yuv_to_rgb_pixel(y_val, u_val, v_val);
                    dst[dst_idx] = r;
                    dst[dst_idx + 1] = g;
                    dst[dst_idx + 2] = b;
                    dst_idx += 3;
                }
            }
        }
        180 => {
            let mut dst_idx = 0;
            for out_y in 0..out_h {
                let src_y = height.saturating_sub(1).saturating_sub(out_y);
                let uv_y = src_y >> 1;
                let y_row_start = src_y * y_row_stride;
                let u_row_start = uv_y * u_row_stride;
                let v_row_start = uv_y * v_row_stride;

                for out_x in 0..out_w {
                    let src_x = width.saturating_sub(1).saturating_sub(out_x);
                    let uv_x = src_x >> 1;

                    let y_idx = y_row_start + src_x;
                    let y_val = if y_idx < y_len { y_plane[y_idx] } else { 0 };

                    let u_idx = u_row_start + uv_x * u_px_stride;
                    let u_val = if has_u && u_idx < u_len { u_plane[u_idx] } else { 128 };

                    let v_idx = v_row_start + uv_x * v_px_stride;
                    let v_val = if has_v && v_idx < v_len { v_plane[v_idx] } else { 128 };

                    let (r, g, b) = yuv_to_rgb_pixel(y_val, u_val, v_val);
                    dst[dst_idx] = r;
                    dst[dst_idx + 1] = g;
                    dst[dst_idx + 2] = b;
                    dst_idx += 3;
                }
            }
        }
        270 => {
            // Clockwise 270 deg (counter-clockwise 90 deg)
            let mut dst_idx = 0;
            for out_y in 0..out_h {
                let src_x = width.saturating_sub(1).saturating_sub(out_y);
                let uv_x = src_x >> 1;
                let uv_u_x_offset = uv_x * u_px_stride;
                let uv_v_x_offset = uv_x * v_px_stride;

                for out_x in 0..out_w {
                    let src_y = out_x;
                    let y_idx = src_y * y_row_stride + src_x;
                    let y_val = if y_idx < y_len { y_plane[y_idx] } else { 0 };

                    let uv_y = src_y >> 1;
                    let u_idx = uv_y * u_row_stride + uv_u_x_offset;
                    let u_val = if has_u && u_idx < u_len { u_plane[u_idx] } else { 128 };

                    let v_idx = uv_y * v_row_stride + uv_v_x_offset;
                    let v_val = if has_v && v_idx < v_len { v_plane[v_idx] } else { 128 };

                    let (r, g, b) = yuv_to_rgb_pixel(y_val, u_val, v_val);
                    dst[dst_idx] = r;
                    dst[dst_idx + 1] = g;
                    dst[dst_idx + 2] = b;
                    dst_idx += 3;
                }
            }
        }
        _ => {
            // 0 deg
            let mut dst_idx = 0;
            for out_y in 0..out_h {
                let src_y = out_y;
                let uv_y = src_y >> 1;
                let y_row_start = src_y * y_row_stride;
                let u_row_start = uv_y * u_row_stride;
                let v_row_start = uv_y * v_row_stride;

                for out_x in 0..out_w {
                    let src_x = out_x;
                    let uv_x = src_x >> 1;

                    let y_idx = y_row_start + src_x;
                    let y_val = if y_idx < y_len { y_plane[y_idx] } else { 0 };

                    let u_idx = u_row_start + uv_x * u_px_stride;
                    let u_val = if has_u && u_idx < u_len { u_plane[u_idx] } else { 128 };

                    let v_idx = v_row_start + uv_x * v_px_stride;
                    let v_val = if has_v && v_idx < v_len { v_plane[v_idx] } else { 128 };

                    let (r, g, b) = yuv_to_rgb_pixel(y_val, u_val, v_val);
                    dst[dst_idx] = r;
                    dst[dst_idx + 1] = g;
                    dst[dst_idx + 2] = b;
                    dst_idx += 3;
                }
            }
        }
    }

    (out_w, out_h)
}

/// Converts YUV_420_888 planes (with arbitrary row strides and pixel strides) to an upright RGB8 buffer.
/// Rotates according to rotation_degrees (0, 90, 180, 270).
/// Returns: (rgb_bytes, oriented_width, oriented_height)
pub fn yuv420_to_rgb(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    width: usize,
    height: usize,
    y_row_stride: usize,
    u_row_stride: usize,
    v_row_stride: usize,
    u_pixel_stride: usize,
    v_pixel_stride: usize,
    rotation_degrees: i32,
) -> (Vec<u8>, usize, usize) {
    let mut dst = Vec::new();
    let (w, h) = yuv420_to_rgb_into(
        y_plane,
        u_plane,
        v_plane,
        width,
        height,
        y_row_stride,
        u_row_stride,
        v_row_stride,
        u_pixel_stride,
        v_pixel_stride,
        rotation_degrees,
        &mut dst,
    );
    (dst, w, h)
}

/// Encodes an RGB8 buffer to JPEG bytes.
pub fn encode_rgb_to_jpeg(
    rgb: &[u8],
    width: usize,
    height: usize,
    quality: u8,
) -> Result<Vec<u8>, String> {
    if rgb.is_empty() || width == 0 || height == 0 {
        return Err("Dữ liệu RGB rỗng".to_string());
    }

    let mut output = Cursor::new(Vec::with_capacity(width * height / 3));
    let encoder = JpegEncoder::new_with_quality(&mut output, quality.clamp(10, 100));

    encoder
        .write_image(rgb, width as u32, height as u32, ColorType::Rgb8.into())
        .map_err(|e| format!("Lỗi mã hóa JPEG: {}", e))?;

    Ok(output.into_inner())
}

/// Encodes a tightly packed grayscale Y plane to JPEG bytes.
pub fn encode_packed_y_to_jpeg(
    packed_y: &[u8],
    width: usize,
    height: usize,
    quality: u8,
) -> Result<Vec<u8>, String> {
    if packed_y.is_empty() || width == 0 || height == 0 {
        return Err("Dữ liệu khung hình rỗng".to_string());
    }

    let mut output = Cursor::new(Vec::with_capacity(width * height / 4));
    let encoder = JpegEncoder::new_with_quality(&mut output, quality.clamp(10, 100));

    encoder
        .write_image(packed_y, width as u32, height as u32, ColorType::L8.into())
        .map_err(|e| format!("Lỗi mã hóa JPEG: {}", e))?;

    Ok(output.into_inner())
}

/// Encodes a grayscale Y plane to JPEG bytes, handling row_stride and 0 rotation.
pub fn encode_y_plane_to_jpeg(
    y_plane: &[u8],
    width: usize,
    height: usize,
    row_stride: usize,
    quality: u8,
) -> Result<Vec<u8>, String> {
    let (packed, w, h) = pack_and_rotate_y_plane(y_plane, width, height, row_stride, 0);
    encode_packed_y_to_jpeg(&packed, w, h, quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yuv_to_rgb_pixel_colors() {
        // Pure black
        let (r, g, b) = yuv_to_rgb_pixel(0, 128, 128);
        assert_eq!((r, g, b), (0, 0, 0));

        // Pure white
        let (r, g, b) = yuv_to_rgb_pixel(255, 128, 128);
        assert_eq!((r, g, b), (255, 255, 255));

        // Standard BT.601 Red: Y=76, U=85, V=255
        let (r, g, b) = yuv_to_rgb_pixel(76, 85, 255);
        assert!(r >= 250);
        assert!(g <= 5);
        assert!(b <= 5);

        // Standard BT.601 Green: Y=150, U=44, V=21
        let (r, g, b) = yuv_to_rgb_pixel(150, 44, 21);
        assert!(r <= 5);
        assert!(g >= 250);
        assert!(b <= 5);

        // Standard BT.601 Blue: Y=29, U=255, V=107
        let (r, g, b) = yuv_to_rgb_pixel(29, 255, 107);
        assert!(r <= 5);
        assert!(g <= 5);
        assert!(b >= 250);
    }

    #[test]
    fn test_yuv420_to_rgb_nv21_format() {
        // 2x2 image, NV21 format (U and V interleaved with pixel_stride 2)
        let w = 2;
        let h = 2;
        // Y plane (all red: Y=76)
        let y_plane = vec![76u8, 76u8, 76u8, 76u8];
        // In NV21, VU are interleaved: [V0, U0]
        let vu_buffer = vec![255u8, 85u8]; // V=255, U=85

        let (rgb, out_w, out_h) = yuv420_to_rgb(
            &y_plane,
            &vu_buffer[1..], // U starts at index 1
            &vu_buffer[0..], // V starts at index 0
            w,
            h,
            w,
            2,
            2,
            2, // pixel_stride 2
            2, // pixel_stride 2
            0,
        );

        assert_eq!(out_w, 2);
        assert_eq!(out_h, 2);
        assert_eq!(rgb.len(), 12);
        for pixel in 0..4 {
            let r = rgb[pixel * 3];
            let g = rgb[pixel * 3 + 1];
            let b = rgb[pixel * 3 + 2];
            assert!(r >= 250);
            assert!(g <= 5);
            assert!(b <= 5);
        }
    }

    #[test]
    fn test_yuv420_to_rgb_rotations() {
        // 4x2 test pattern: top half red (76, 85, 255), bottom half green (150, 44, 21)
        let w = 4;
        let h = 2;
        let y_plane = vec![
            76, 76, 76, 76, // row 0: red
            150, 150, 150, 150, // row 1: green
        ];
        let u_plane = vec![85, 85]; // 1 chroma row (h/2 = 1, w/2 = 2)
        let v_plane = vec![255, 255];

        // 0 deg rotation
        let (rgb_0, w0, h0) = yuv420_to_rgb(&y_plane, &u_plane, &v_plane, w, h, w, 2, 2, 1, 1, 0);
        assert_eq!((w0, h0), (4, 2));
        assert!(rgb_0[0] >= 250); // top left is red

        // 90 deg rotation (clockwise)
        let (_rgb_90, w90, h90) = yuv420_to_rgb(&y_plane, &u_plane, &v_plane, w, h, w, 2, 2, 1, 1, 90);
        assert_eq!((w90, h90), (2, 4));

        // 180 deg rotation
        let (_rgb_180, w180, h180) = yuv420_to_rgb(&y_plane, &u_plane, &v_plane, w, h, w, 2, 2, 1, 1, 180);
        assert_eq!((w180, h180), (4, 2));

        // 270 deg rotation
        let (_rgb_270, w270, h270) = yuv420_to_rgb(&y_plane, &u_plane, &v_plane, w, h, w, 2, 2, 1, 1, 270);
        assert_eq!((w270, h270), (2, 4));
    }

    #[test]
    fn test_encode_rgb_to_jpeg() {
        let width = 640;
        let height = 480;
        let mut rgb = vec![0u8; width * height * 3];
        // Red test pattern
        for i in 0..(width * height) {
            rgb[i * 3] = 255;
            rgb[i * 3 + 1] = 0;
            rgb[i * 3 + 2] = 0;
        }
        let start = std::time::Instant::now();
        let res = encode_rgb_to_jpeg(&rgb, width, height, 75);
        let elapsed = start.elapsed();
        println!("640x480 RGB JPEG encode took {:?}", elapsed);
        assert!(res.is_ok());
        let jpeg = res.unwrap();
        assert!(jpeg.len() > 100);
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);
    }

    #[test]
    fn test_encode_y_plane_to_jpeg() {
        let width = 320;
        let height = 240;
        let y_plane = vec![128u8; width * height];
        let res = encode_y_plane_to_jpeg(&y_plane, width, height, width, 70);
        assert!(res.is_ok());
        let jpeg = res.unwrap();
        // JPEG magic bytes: 0xFF, 0xD8
        assert!(jpeg.len() > 100);
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);
    }

    #[test]
    fn test_pack_and_rotate_0_deg() {
        let src = vec![1, 2, 3, 4, 5, 6];
        let (dst, w, h) = pack_and_rotate_y_plane(&src, 3, 2, 3, 0);
        assert_eq!(w, 3);
        assert_eq!(h, 2);
        assert_eq!(dst, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_pack_and_rotate_90_deg() {
        // [1, 2, 3]
        // [4, 5, 6]
        let src = vec![1, 2, 3, 4, 5, 6];
        let (dst, w, h) = pack_and_rotate_y_plane(&src, 3, 2, 3, 90);
        assert_eq!(w, 2);
        assert_eq!(h, 3);
        // Rotated 90 deg clockwise:
        // [4, 1]
        // [5, 2]
        // [6, 3]
        assert_eq!(dst, vec![4, 1, 5, 2, 6, 3]);
    }

    #[test]
    fn test_pack_and_rotate_180_deg() {
        let src = vec![1, 2, 3, 4, 5, 6];
        let (dst, w, h) = pack_and_rotate_y_plane(&src, 3, 2, 3, 180);
        assert_eq!(w, 3);
        assert_eq!(h, 2);
        assert_eq!(dst, vec![6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn test_pack_and_rotate_270_deg() {
        let src = vec![1, 2, 3, 4, 5, 6];
        let (dst, w, h) = pack_and_rotate_y_plane(&src, 3, 2, 3, 270);
        assert_eq!(w, 2);
        assert_eq!(h, 3);
        // Rotated 270 deg clockwise (90 counter-clockwise):
        // [3, 6]
        // [2, 5]
        // [1, 4]
        assert_eq!(dst, vec![3, 6, 2, 5, 1, 4]);
    }

    #[test]
    fn test_pack_and_rotate_stride_padding() {
        // width=2, height=2, row_stride=4 (padding of 2 bytes per row)
        let src = vec![10, 20, 0, 0, 30, 40, 0, 0];
        let (dst, w, h) = pack_and_rotate_y_plane(&src, 2, 2, 4, 0);
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        assert_eq!(dst, vec![10, 20, 30, 40]);
    }

    static TEST_MUTEX: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    #[test]
    fn test_device_status_lifecycle() {
        let _guard = TEST_MUTEX.lock();
        let temp_dir = std::env::temp_dir().join(format!("watchcam_test_{}", rand::random::<u32>()));
        let db_file = temp_dir.join("test.db");
        let _ = WatchCamEngine::initialize(&temp_dir, &db_file, 19876);

        // Before update, default is empty or None
        let initial_json = WatchCamEngine::get_device_status_json();
        assert_eq!(initial_json, "{}");

        let status_payload = r#"{
            "battery": {
                "level": 88,
                "isCharging": true,
                "chargingSource": "AC",
                "health": "Tốt",
                "temperature": 33.2
            },
            "thermal": {
                "temperature": 33.2,
                "status": "Bình thường"
            },
            "ram": {
                "usedBytes": 1500000000,
                "totalBytes": 3000000000,
                "availableBytes": 1500000000,
                "usedPercent": 50
            },
            "storage": {
                "freeBytes": 16000000000,
                "totalBytes": 32000000000,
                "snapshotsBytes": 12000000
            },
            "wifi": {
                "connected": true,
                "ssid": "WatchCam_WiFi",
                "rssi": -52,
                "signalQuality": "Xuất sắc",
                "linkSpeedMbps": 144
            },
            "uptime": {
                "deviceUptimeSeconds": 10000,
                "serviceUptimeSeconds": 1200
            },
            "timestamp": 1731000000000
        }"#;

        let ok = WatchCamEngine::update_device_status(status_payload);
        assert!(ok);

        let retrieved = WatchCamEngine::get_device_status_json();
        assert!(retrieved.contains("WatchCam_WiFi"));
        assert!(retrieved.contains("144"));

        let overall = WatchCamEngine::get_status_json();
        assert!(overall.contains("WatchCam_WiFi"));
        assert!(overall.contains("\"device\":"));

        WatchCamEngine::stop();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_revoke_all_sessions_clears_sqlite_even_when_engine_stopped() {
        let _guard = TEST_MUTEX.lock();
        let temp_dir = std::env::temp_dir().join(format!("watchcam_test_{}", rand::random::<u32>()));
        let db_file = temp_dir.join("test_revocation.db");
        let _ = WatchCamEngine::initialize(&temp_dir, &db_file, 19877);

        // Pair a device while engine is running
        let code = WatchCamEngine::get_pairing_code();
        let token = {
            let lock = ENGINE.lock();
            let engine = lock.as_ref().unwrap();
            engine.app_state.auth.verify_pairing_code(&code).unwrap()
        };

        // Confirm token is in DB
        {
            let conn = rusqlite::Connection::open(&db_file).unwrap();
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM paired_devices", [], |r| r.get(0)).unwrap();
            assert_eq!(count, 1);
        }

        // Now STOP the surveillance engine (simulating service stopped)
        WatchCamEngine::stop();

        // While engine is stopped, user taps "Revoke all sessions" on tablet
        WatchCamEngine::revoke_all_sessions_with_db(Some(&db_file));

        // SQLite paired_devices table MUST be cleared
        {
            let conn = rusqlite::Connection::open(&db_file).unwrap();
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM paired_devices", [], |r| r.get(0)).unwrap();
            assert_eq!(count, 0, "Paired devices must be wiped from SQLite even when engine is stopped");
        }

        // Restart engine: token must NOT be loaded back
        let _ = WatchCamEngine::initialize(&temp_dir, &db_file, 19877);
        {
            let lock = ENGINE.lock();
            let engine = lock.as_ref().unwrap();
            assert!(!engine.app_state.auth.validate_token(&token));
        }

        WatchCamEngine::stop();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_update_email_config() {
        let _guard = TEST_MUTEX.lock();
        let temp_dir = std::env::temp_dir().join(format!("watchcam_test_email_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let db_file = temp_dir.join("test.db");
        let _ = std::fs::create_dir_all(&temp_dir);

        let init_res = WatchCamEngine::initialize(&temp_dir, &db_file, 19878);
        assert!(init_res.is_ok());

        WatchCamEngine::update_email_config(
            true,
            "smtp.gmail.com",
            587,
            "ban@gmail.com",
            "abcdefghijklmnop",
            "nguoinhan@gmail.com",
        );

        let status_json = WatchCamEngine::get_status_json();
        let status: serde_json::Value = serde_json::from_str(&status_json).unwrap();
        assert_eq!(status["emailAlertsEnabled"], true);
        assert_eq!(status["smtpHost"], "smtp.gmail.com");
        assert_eq!(status["smtpPort"], 587);
        assert_eq!(status["smtpUsername"], "ban@gmail.com");
        assert_eq!(status["smtpPassword"], "abcdefghijklmnop");
        assert_eq!(status["smtpRecipient"], "nguoinhan@gmail.com");

        WatchCamEngine::stop();
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
