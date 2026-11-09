use crate::auth::AuthManager;
use crate::config::SurveillanceConfig;
use crate::storage::EventDatabase;
use crate::stream::{AudioBroadcaster, FrameBroadcaster};
use crate::web_assets::{APP_JS, INDEX_HTML, STYLE_CSS};

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path as AxumPath, Query, State, WebSocketUpgrade};
use axum::http::header::{HeaderMap, HeaderValue, CONTENT_TYPE, SET_COOKIE};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use futures::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<SurveillanceConfig>>,
    pub auth: AuthManager,
    pub broadcaster: FrameBroadcaster,
    pub audio_broadcaster: AudioBroadcaster,
    pub db: Arc<EventDatabase>,
    pub ws_sender: broadcast::Sender<String>,
    pub is_monitoring: Arc<AtomicBool>,
    pub switch_camera_requested: Arc<AtomicBool>,
    pub resolution_change_requested: Arc<AtomicU8>,
    pub test_email_requested: Arc<AtomicBool>,
    pub device_status: Arc<RwLock<Option<serde_json::Value>>>,
}

#[derive(Deserialize)]
pub struct PairRequest {
    pub code: String,
}

#[derive(Serialize)]
pub struct PairResponse {
    pub success: bool,
    pub token: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PatchConfigRequest {
    pub motion_enabled: Option<bool>,
    pub sensitivity: Option<u8>,
    pub stream_quality: Option<u8>,
    pub target_stream_fps: Option<u32>,
    pub switch_camera: Option<bool>,
    pub resolution: Option<String>,
    pub cooldown_seconds: Option<u64>,
    pub email_alerts_enabled: Option<bool>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_recipient: Option<String>,
    pub https_port: Option<u16>,
}

impl PatchConfigRequest {
    pub fn from_value(val: &serde_json::Value) -> Self {
        let extract_bool = |keys: &[&str]| -> Option<bool> {
            for &k in keys {
                if let Some(v) = val.get(k) {
                    if let Some(b) = v.as_bool() {
                        return Some(b);
                    }
                    if let Some(n) = v.as_i64() {
                        if n == 1 {
                            return Some(true);
                        }
                        if n == 0 {
                            return Some(false);
                        }
                    }
                    if let Some(s) = v.as_str() {
                        if s.eq_ignore_ascii_case("true") || s == "1" {
                            return Some(true);
                        }
                        if s.eq_ignore_ascii_case("false") || s == "0" {
                            return Some(false);
                        }
                    }
                }
            }
            None
        };

        let extract_str = |keys: &[&str]| -> Option<String> {
            for &k in keys {
                if let Some(v) = val.get(k) {
                    if let Some(s) = v.as_str() {
                        return Some(s.to_string());
                    }
                }
            }
            None
        };

        let extract_u64 = |keys: &[&str]| -> Option<u64> {
            for &k in keys {
                if let Some(v) = val.get(k) {
                    if let Some(n) = v.as_u64() {
                        return Some(n);
                    }
                    if let Some(i) = v.as_i64() {
                        if i >= 0 {
                            return Some(i as u64);
                        }
                    }
                    if let Some(f) = v.as_f64() {
                        if f >= 0.0 {
                            return Some(f as u64);
                        }
                    }
                    if let Some(s) = v.as_str() {
                        if let Ok(n) = s.trim().parse::<u64>() {
                            return Some(n);
                        }
                    }
                }
            }
            None
        };

        Self {
            motion_enabled: extract_bool(&["motionEnabled", "motion_enabled"]),
            sensitivity: extract_u64(&["sensitivity"]).map(|v| v as u8),
            stream_quality: extract_u64(&["streamQuality", "stream_quality"]).map(|v| v as u8),
            target_stream_fps: extract_u64(&["targetStreamFps", "target_stream_fps"]).map(|v| v as u32),
            switch_camera: extract_bool(&["switchCamera", "switch_camera"]),
            resolution: extract_str(&["resolution"]),
            cooldown_seconds: extract_u64(&["cooldownSeconds", "cooldown_seconds"]),
            email_alerts_enabled: extract_bool(&["emailAlertsEnabled", "email_alerts_enabled"]),
            smtp_host: extract_str(&["smtpHost", "smtp_host"]),
            smtp_port: extract_u64(&["smtpPort", "smtp_port"]).map(|v| v as u16),
            smtp_username: extract_str(&["smtpUsername", "smtp_username"]),
            smtp_password: extract_str(&["smtpPassword", "smtp_password"]),
            smtp_recipient: extract_str(&["smtpRecipient", "smtp_recipient"]),
            https_port: extract_u64(&["httpsPort", "https_port"]).map(|v| v as u16),
        }
    }
}

impl<'de> serde::Deserialize<'de> for PatchConfigRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = serde_json::Value::deserialize(deserializer)?;
        Ok(Self::from_value(&val))
    }
}

#[derive(Deserialize, Default)]
pub struct WsAuthQuery {
    pub token: Option<String>,
}

fn extract_all_session_tokens(headers: &HeaderMap) -> Vec<String> {
    let mut tokens = Vec::new();
    // 1. Try Authorization: Bearer XYZ
    if let Some(auth_hdr) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_hdr.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                let token = token.trim();
                if !token.is_empty() {
                    tokens.push(token.to_string());
                }
            }
        }
    }
    // 2. Try Cookie header: watchcam_session=XYZ
    if let Some(cookie_hdr) = headers.get(axum::http::header::COOKIE) {
        if let Ok(cookie_str) = cookie_hdr.to_str() {
            for part in cookie_str.split(';') {
                let part = part.trim();
                if let Some(val) = part.strip_prefix("watchcam_session=") {
                    let val = val.trim();
                    if !val.is_empty() {
                        tokens.push(val.to_string());
                    }
                }
            }
        }
    }
    tokens
}

fn is_authenticated_request(state: &AppState, headers: &HeaderMap, query_token: Option<&str>) -> bool {
    // 1. Check query parameter token
    if let Some(t) = query_token {
        let t = t.trim();
        if !t.is_empty() && state.auth.validate_token(t) {
            return true;
        }
    }
    // 2. Check header and cookie tokens
    for token in extract_all_session_tokens(headers) {
        if state.auth.validate_token(&token) {
            return true;
        }
    }
    false
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Web Dashboard Assets
        .route("/", get(handle_index))
        .route("/index.html", get(handle_index))
        .route("/style.css", get(handle_css))
        .route("/app.js", get(handle_js))
        // Authentication
        .route("/api/auth/pair", post(handle_pair))
        .route("/api/auth/logout", post(handle_logout))
        // Status and Config
        .route("/api/status", get(handle_status))
        .route("/api/device/status", get(handle_device_status))
        .route("/api/config", get(handle_get_config).patch(handle_patch_config))
        // Video Stream and Snapshot
        .route("/stream.mjpeg", get(handle_mjpeg_stream))
        .route("/api/snapshot", get(handle_snapshot))
        // Event History, Snapshots and Video Clips
        .route("/api/events", get(handle_list_events))
        .route("/api/events/:id", get(handle_get_event).delete(handle_delete_event))
        .route("/api/events/:id/snapshot", get(handle_event_snapshot))
        .route("/api/events/:id/video", get(handle_event_video).head(handle_event_video))
        .route("/api/events/clear", post(handle_clear_events))
        // Camera Controls & Email Test
        .route("/api/camera/start", post(handle_camera_start))
        .route("/api/camera/stop", post(handle_camera_stop))
        .route("/api/email/test", post(handle_test_email))
        // WebSocket
        .route("/ws/events", get(handle_ws_upgrade))
        // Audio Streaming & Two-Way Intercom
        .route("/ws/audio", get(handle_ws_audio))
        .route("/stream.wav", get(handle_wav_stream))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// Handler Implementations

async fn handle_index() -> impl IntoResponse {
    Html(INDEX_HTML)
}

async fn handle_css() -> impl IntoResponse {
    ([(CONTENT_TYPE, "text/css; charset=utf-8")], STYLE_CSS)
}

async fn handle_js() -> impl IntoResponse {
    ([(CONTENT_TYPE, "application/javascript; charset=utf-8")], APP_JS)
}

async fn handle_pair(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<PairRequest>,
) -> impl IntoResponse {
    let device_name = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok());

    match state.auth.verify_pairing_code_with_device(&payload.code, device_name) {
        Ok(token) => {
            let cookie_val = format!(
                "watchcam_session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=31536000",
                token
            );
            let mut headers = HeaderMap::new();
            if let Ok(hdr) = HeaderValue::from_str(&cookie_val) {
                headers.insert(SET_COOKIE, hdr);
            }
            (
                StatusCode::OK,
                headers,
                axum::Json(PairResponse {
                    success: true,
                    token: Some(token),
                    message: None,
                }),
            )
        }
        Err(err) => (
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            axum::Json(PairResponse {
                success: false,
                token: None,
                message: Some(err.to_string()),
            }),
        ),
    }
}

async fn handle_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> impl IntoResponse {
    if let Some(t) = query.token {
        let t = t.trim();
        if !t.is_empty() {
            state.auth.revoke_token(t);
        }
    }
    for token in extract_all_session_tokens(&headers) {
        state.auth.revoke_token(&token);
    }
    let clear_cookie = "watchcam_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0";
    let mut resp_headers = HeaderMap::new();
    if let Ok(hdr) = HeaderValue::from_str(clear_cookie) {
        resp_headers.insert(SET_COOKIE, hdr);
    }
    (StatusCode::OK, resp_headers, "Đã đăng xuất")
}

async fn handle_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> axum::Json<serde_json::Value> {
    let is_auth = is_authenticated_request(&state, &headers, query.token.as_deref());
    let cfg = state.config.read().clone();
    let total_events = state.db.count_events().unwrap_or(0);
    let dev = state.device_status.read().clone();

    let status = serde_json::json!({
        "cameraName": if cfg.camera_id == "front" { "Camera Trước" } else { "Camera Sau" },
        "isMonitoring": state.is_monitoring.load(Ordering::Relaxed),
        "actualFps": state.broadcaster.get_actual_fps(),
        "viewerCount": state.broadcaster.get_viewer_count(),
        "audioLevel": state.audio_broadcaster.get_audio_level(),
        "audioViewerCount": state.audio_broadcaster.get_viewer_count(),
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
        "totalEvents": total_events,
        "serverPort": cfg.server_port,
        "httpsPort": cfg.https_port,
        "isAuthenticated": is_auth,
        "device": dev,
    });

    axum::Json(status)
}

async fn handle_device_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let dev = state.device_status.read().clone();
    Ok(axum::Json(dev.unwrap_or_else(|| serde_json::json!({}))))
}

async fn handle_get_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let cfg = state.config.read().clone();
    Ok(axum::Json(cfg))
}

async fn handle_patch_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
    axum::Json(payload): axum::Json<serde_json::Value>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let patch = PatchConfigRequest::from_value(&payload);
    let mut cfg = state.config.write();
    if let Some(m) = patch.motion_enabled {
        cfg.motion_enabled = m;
    }
    if let Some(s) = patch.sensitivity {
        cfg.apply_sensitivity(s);
    }
    if let Some(q) = patch.stream_quality {
        cfg.stream_quality = q.clamp(10, 100);
    }
    if let Some(fps) = patch.target_stream_fps {
        cfg.target_stream_fps = fps.clamp(5, 30);
    }
    if let Some(true) = patch.switch_camera {
        state.switch_camera_requested.store(true, Ordering::Relaxed);
    }
    if let Some(r) = patch.resolution {
        if r == "720p" {
            state.resolution_change_requested.store(2, Ordering::Relaxed);
            cfg.resolution = "720p".to_string();
        } else if r == "480p" {
            state.resolution_change_requested.store(1, Ordering::Relaxed);
            cfg.resolution = "480p".to_string();
        }
    }
    if let Some(cd) = patch.cooldown_seconds {
        cfg.cooldown_seconds = cd.clamp(1, 60);
    }
    if let Some(ea) = patch.email_alerts_enabled {
        cfg.email_alerts_enabled = ea;
    }
    if let Some(h) = patch.smtp_host {
        let trimmed = h.trim();
        if !trimmed.is_empty() {
            cfg.smtp_host = trimmed.to_string();
        }
    }
    if let Some(p) = patch.smtp_port {
        if p > 0 {
            cfg.smtp_port = p;
        }
    }
    if let Some(u) = patch.smtp_username {
        cfg.smtp_username = u.trim().to_string();
    }
    if let Some(pw) = patch.smtp_password {
        cfg.smtp_password = pw.trim().to_string();
    }
    if let Some(rcpt) = patch.smtp_recipient {
        cfg.smtp_recipient = rcpt.trim().to_string();
    }
    if let Some(hp) = patch.https_port {
        if hp > 0 {
            cfg.https_port = hp;
        }
    }

    // Broadcast config update to connected WebSocket clients
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
    let _ = state.ws_sender.send(ws_msg.to_string());

    Ok(axum::Json(cfg.clone()))
}

struct ViewerGuard(FrameBroadcaster);
impl ViewerGuard {
    fn new(broadcaster: FrameBroadcaster) -> Self {
        broadcaster.increment_viewer();
        Self(broadcaster)
    }
}
impl Drop for ViewerGuard {
    fn drop(&mut self) {
        self.0.decrement_viewer();
    }
}

async fn handle_mjpeg_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<Response, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let validated_token = {
        let mut found = None;
        if let Some(ref t) = query.token {
            let t = t.trim();
            if !t.is_empty() && state.auth.validate_token(t) {
                found = Some(t.to_string());
            }
        }
        if found.is_none() {
            for token in extract_all_session_tokens(&headers) {
                if state.auth.validate_token(&token) {
                    found = Some(token);
                    break;
                }
            }
        }
        found
    };

    let broadcaster = state.broadcaster.clone();
    let mut rx = broadcaster.subscribe();
    let auth = state.auth.clone();

    let stream = async_stream::stream! {
        let _guard = ViewerGuard::new(broadcaster.clone());
        let mut frame_count = 0u64;

        // Emit current cached frame immediately if available to reduce initial load latency
        let initial_frame = {
            let guard = rx.borrow_and_update();
            guard.clone()
        };

        if let Some(frame) = initial_frame {
            let header = format!(
                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                frame.jpeg_data.len()
            );
            let mut chunk = Vec::with_capacity(header.len() + frame.jpeg_data.len() + 2);
            chunk.extend_from_slice(header.as_bytes());
            chunk.extend_from_slice(&frame.jpeg_data);
            chunk.extend_from_slice(b"\r\n");
            yield Ok::<_, std::io::Error>(bytes::Bytes::from(chunk));
        }

        loop {
            // Wait for new frame
            if rx.changed().await.is_err() {
                break;
            }

            frame_count += 1;
            if frame_count % 30 == 0 {
                if let Some(ref t) = validated_token {
                    if !auth.validate_token(t) {
                        break;
                    }
                }
            }

            let frame_opt = {
                let guard = rx.borrow();
                guard.clone()
            };

            if let Some(frame) = frame_opt {
                let header = format!(
                    "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                    frame.jpeg_data.len()
                );
                let mut chunk = Vec::with_capacity(header.len() + frame.jpeg_data.len() + 2);
                chunk.extend_from_slice(header.as_bytes());
                chunk.extend_from_slice(&frame.jpeg_data);
                chunk.extend_from_slice(b"\r\n");
                yield Ok::<_, std::io::Error>(bytes::Bytes::from(chunk));
            }
        }
    };

    let body = Body::from_stream(stream);
    let mut response = Response::new(body);
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("multipart/x-mixed-replace; boundary=frame"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );

    Ok(response)
}

async fn handle_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    if let Some(frame) = state.broadcaster.get_latest_frame() {
        let mut resp = Response::new(Body::from((*frame.jpeg_data).clone()));
        resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
        Ok(resp)
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

async fn handle_list_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    match state.db.get_events(50, 0) {
        Ok(events) => Ok(axum::Json(events)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_get_event(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    match state.db.get_event(&id) {
        Ok(Some(ev)) => Ok(axum::Json(ev)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_event_snapshot(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    match state.db.get_event(&id) {
        Ok(Some(ev)) => {
            if let Ok(bytes) = fs::read(&ev.snapshot_path) {
                let mut resp = Response::new(Body::from(bytes));
                resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
                Ok(resp)
            } else {
                Err(StatusCode::NOT_FOUND)
            }
        }
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_event_video(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<Response, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let (event_id, video_path) = match state.db.get_event(&id) {
        Ok(Some(ev)) => {
            let mut found = None;
            if let Some(ref vp) = ev.video_path {
                let p = std::path::PathBuf::from(vp);
                if p.is_file() {
                    found = Some(p);
                }
            }
            if found.is_none() {
                let snap_path = std::path::Path::new(&ev.snapshot_path);
                let parent = snap_path.parent().unwrap_or(std::path::Path::new("."));
                let p = parent.join(format!("motion_{}.mp4", ev.id));
                if p.is_file() {
                    found = Some(p);
                }
            }
            if found.is_none() {
                let p = state.db.storage_dir.join(format!("motion_{}.mp4", ev.id));
                if p.is_file() {
                    found = Some(p);
                }
            }
            match found {
                Some(p) => (ev.id, p),
                None => return Err(StatusCode::NOT_FOUND),
            }
        }
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let metadata = fs::metadata(&video_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let total_len = metadata.len() as usize;
    if total_len == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    let cd_val = format!("inline; filename=\"motion_{}.mp4\"", event_id);

    // Support HTTP Range requests
    if let Some(range_header) = headers.get(axum::http::header::RANGE) {
        if let Ok(range_str) = range_header.to_str() {
            if let Some(spec) = range_str.strip_prefix("bytes=") {
                let parts: Vec<&str> = spec.split('-').collect();
                let start: usize = parts[0].parse().unwrap_or(0);
                let end: usize = if parts.len() > 1 && !parts[1].is_empty() {
                    parts[1].parse().unwrap_or(total_len.saturating_sub(1)).min(total_len.saturating_sub(1))
                } else {
                    total_len.saturating_sub(1)
                };

                if start <= end && start < total_len {
                    let chunk_len = end - start + 1;
                    use std::io::{Read, Seek, SeekFrom};
                    let mut file = fs::File::open(&video_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                    file.seek(SeekFrom::Start(start as u64)).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                    let mut chunk_bytes = vec![0u8; chunk_len];
                    file.read_exact(&mut chunk_bytes).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                    let mut resp = Response::new(Body::from(chunk_bytes));
                    *resp.status_mut() = StatusCode::PARTIAL_CONTENT;
                    resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("video/mp4"));
                    resp.headers_mut().insert(axum::http::header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
                    let cr_val = format!("bytes {}-{}/{}", start, end, total_len);
                    if let Ok(hdr) = HeaderValue::from_str(&cr_val) {
                        resp.headers_mut().insert(axum::http::header::CONTENT_RANGE, hdr);
                    }
                    resp.headers_mut().insert(axum::http::header::CONTENT_LENGTH, HeaderValue::from(chunk_len));
                    if let Ok(hdr) = HeaderValue::from_str(&cd_val) {
                        resp.headers_mut().insert(axum::http::header::CONTENT_DISPOSITION, hdr);
                    }
                    return Ok(resp);
                }
            }
        }
    }

    // Full 200 OK response
    let file_bytes = fs::read(&video_path).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut resp = Response::new(Body::from(file_bytes));
    resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("video/mp4"));
    resp.headers_mut().insert(axum::http::header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    resp.headers_mut().insert(axum::http::header::CONTENT_LENGTH, HeaderValue::from(total_len));
    if let Ok(hdr) = HeaderValue::from_str(&cd_val) {
        resp.headers_mut().insert(axum::http::header::CONTENT_DISPOSITION, hdr);
    }
    Ok(resp)
}

async fn handle_delete_event(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    match state.db.delete_event(&id) {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_clear_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    match state.db.clear_all_events() {
        Ok(cnt) => Ok(axum::Json(serde_json::json!({ "cleared": cnt }))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn handle_camera_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    state.is_monitoring.store(true, Ordering::Relaxed);
    Ok(StatusCode::OK)
}

async fn handle_camera_stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    state.is_monitoring.store(false, Ordering::Relaxed);
    Ok(StatusCode::OK)
}

async fn handle_test_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    state.test_email_requested.store(true, Ordering::Relaxed);
    Ok(axum::Json(serde_json::json!({
        "success": true,
        "message": "Đã yêu cầu camera gửi email thử nghiệm"
    })))
}

async fn handle_ws_upgrade(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<Response, StatusCode> {
    let authenticated = is_authenticated_request(&state, &headers, query.token.as_deref());

    if !authenticated {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let rx = state.ws_sender.subscribe();
    Ok(ws.on_upgrade(move |socket| handle_ws_session(socket, rx, state)))
}

async fn handle_ws_session(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<String>,
    state: AppState,
) {
    // Send immediate initial status upon WebSocket connection
    let cfg = state.config.read().clone();
    let dev = state.device_status.read().clone();
    let init_status = serde_json::json!({
        "type": "status",
        "status": {
            "cameraName": if cfg.camera_id == "front" { "Camera Trước" } else { "Camera Sau" },
            "isMonitoring": state.is_monitoring.load(Ordering::Relaxed),
            "actualFps": state.broadcaster.get_actual_fps(),
            "viewerCount": state.broadcaster.get_viewer_count(),
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
            "totalEvents": state.db.count_events().unwrap_or(0),
            "serverPort": cfg.server_port,
            "httpsPort": cfg.https_port,
            "isAuthenticated": true,
            "device": dev,
        }
    });
    if socket.send(Message::Text(init_status.to_string())).await.is_err() {
        return;
    }

    // Also send an initial device_status frame if telemetry has arrived
    if let Some(device_val) = dev {
        let dev_msg = serde_json::json!({
            "type": "device_status",
            "device": device_val,
        });
        if socket.send(Message::Text(dev_msg.to_string())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            msg_res = rx.recv() => {
                match msg_res {
                    Ok(payload) => {
                        if socket.send(Message::Text(payload)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            client_msg = socket.recv() => {
                match client_msg {
                    Some(Ok(Message::Ping(p))) => {
                        if socket.send(Message::Pong(p)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

async fn handle_ws_audio(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<Response, StatusCode> {
    let authenticated = is_authenticated_request(&state, &headers, query.token.as_deref());
    if !authenticated {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(ws.on_upgrade(move |socket| handle_audio_session(socket, state)))
}

async fn handle_audio_session(socket: WebSocket, state: AppState) {
    state.audio_broadcaster.increment_viewer();
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let mut audio_rx = state.audio_broadcaster.subscribe();

    // Downlink: send PCM chunks from tablet microphone to web client
    let mut send_task = tokio::spawn(async move {
        loop {
            match audio_rx.recv().await {
                Ok(chunk) => {
                    if ws_sender.send(Message::Binary(chunk.to_vec())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Receiver lagged behind sender; continue streaming latest packets
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });

    // Uplink: receive PCM chunks from web client and forward to tablet speaker (intercom)
    let mut recv_task = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Binary(pcm_bytes)) => {
                    if !pcm_bytes.is_empty() {
                        crate::jni_api::dispatch_client_audio(&pcm_bytes);
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => {},
        _ = (&mut recv_task) => {},
    }
    send_task.abort();
    recv_task.abort();

    state.audio_broadcaster.decrement_viewer();
}

struct AudioViewerGuard(crate::stream::AudioBroadcaster);
impl Drop for AudioViewerGuard {
    fn drop(&mut self) {
        self.0.decrement_viewer();
    }
}

async fn handle_wav_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsAuthQuery>,
) -> Result<Response, StatusCode> {
    if !is_authenticated_request(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let mut audio_rx = state.audio_broadcaster.subscribe();
    let broadcaster = state.audio_broadcaster.clone();
    broadcaster.increment_viewer();

    let stream = async_stream::stream! {
        let _guard = AudioViewerGuard(broadcaster);
        let header = crate::stream::generate_wav_header(16000, 1, 16);
        yield Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(&header));

        loop {
            match audio_rx.recv().await {
                Ok(chunk) => {
                    yield Ok::<_, std::io::Error>(chunk);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    resp_headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );

    let body = Body::from_stream(stream);
    let mut response = Response::new(body);
    *response.headers_mut() = resp_headers;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::MotionEvent;

    #[test]
    fn test_patch_config_request_deserialization_camel_case() {
        let json_str = r#"{
            "motionEnabled": false,
            "sensitivity": 85,
            "streamQuality": 90,
            "targetStreamFps": 15,
            "switchCamera": true,
            "resolution": "720p",
            "cooldownSeconds": 12,
            "emailAlertsEnabled": true,
            "smtpHost": "smtp.gmail.com",
            "smtpPort": 465,
            "smtpUsername": "ban@gmail.com",
            "smtpPassword": "secretapppassword",
            "smtpRecipient": "nguoinhan@gmail.com"
        }"#;

        let patch: PatchConfigRequest = serde_json::from_str(json_str).expect("Failed to deserialize camelCase patch");
        assert_eq!(patch.motion_enabled, Some(false));
        assert_eq!(patch.sensitivity, Some(85));
        assert_eq!(patch.stream_quality, Some(90));
        assert_eq!(patch.target_stream_fps, Some(15));
        assert_eq!(patch.switch_camera, Some(true));
        assert_eq!(patch.resolution.as_deref(), Some("720p"));
        assert_eq!(patch.cooldown_seconds, Some(12));
        assert_eq!(patch.email_alerts_enabled, Some(true));
        assert_eq!(patch.smtp_host.as_deref(), Some("smtp.gmail.com"));
        assert_eq!(patch.smtp_port, Some(465));
        assert_eq!(patch.smtp_username.as_deref(), Some("ban@gmail.com"));
        assert_eq!(patch.smtp_password.as_deref(), Some("secretapppassword"));
        assert_eq!(patch.smtp_recipient.as_deref(), Some("nguoinhan@gmail.com"));
    }

    #[test]
    fn test_patch_config_request_deserialization_snake_case() {
        let json_str = r#"{
            "motion_enabled": false,
            "stream_quality": 60,
            "target_stream_fps": 10,
            "switch_camera": false,
            "cooldown_seconds": 7,
            "email_alerts_enabled": false,
            "smtp_host": "smtp.mail.yahoo.com",
            "smtp_port": 587,
            "smtp_username": "user@yahoo.com",
            "smtp_password": "pass",
            "smtp_recipient": "dest@yahoo.com"
        }"#;

        let patch: PatchConfigRequest = serde_json::from_str(json_str).expect("Failed to deserialize snake_case patch");
        assert_eq!(patch.motion_enabled, Some(false));
        assert_eq!(patch.stream_quality, Some(60));
        assert_eq!(patch.target_stream_fps, Some(10));
        assert_eq!(patch.switch_camera, Some(false));
        assert_eq!(patch.cooldown_seconds, Some(7));
        assert_eq!(patch.email_alerts_enabled, Some(false));
        assert_eq!(patch.smtp_host.as_deref(), Some("smtp.mail.yahoo.com"));
        assert_eq!(patch.smtp_port, Some(587));
    }

    #[test]
    fn test_patch_config_request_deserialization_duplicate_and_flexible_keys() {
        // This reproduces the exact scenario that previously caused HTTP 422:
        // Client sends both camelCase and snake_case in the same payload,
        // and string-encoded port numbers like "587".
        let json_str = r#"{
            "emailAlertsEnabled": true,
            "email_alerts_enabled": true,
            "smtpHost": "smtp.gmail.com",
            "smtp_host": "smtp.gmail.com",
            "smtpPort": "587",
            "smtp_port": 587,
            "smtpUsername": "ban@gmail.com",
            "smtp_username": "ban@gmail.com",
            "smtpPassword": "abcd efgh ijkl mnop",
            "smtp_password": "abcd efgh ijkl mnop",
            "smtpRecipient": "nguoinhan@gmail.com",
            "smtp_recipient": "nguoinhan@gmail.com",
            "motionEnabled": true,
            "motion_enabled": true,
            "cooldownSeconds": "15"
        }"#;

        let patch: PatchConfigRequest = serde_json::from_str(json_str)
            .expect("Flexible deserialization must never fail on duplicate or string-number keys");
        assert_eq!(patch.email_alerts_enabled, Some(true));
        assert_eq!(patch.smtp_host.as_deref(), Some("smtp.gmail.com"));
        assert_eq!(patch.smtp_port, Some(587));
        assert_eq!(patch.smtp_username.as_deref(), Some("ban@gmail.com"));
        assert_eq!(patch.smtp_password.as_deref(), Some("abcd efgh ijkl mnop"));
        assert_eq!(patch.smtp_recipient.as_deref(), Some("nguoinhan@gmail.com"));
        assert_eq!(patch.motion_enabled, Some(true));
        assert_eq!(patch.cooldown_seconds, Some(15));
    }

    #[test]
    fn test_patch_config_request_numeric_booleans_and_floats() {
        let json_str = r#"{
            "emailAlertsEnabled": 1,
            "motion_enabled": 0,
            "smtpPort": 587.0,
            "sensitivity": 80.0
        }"#;

        let patch: PatchConfigRequest = serde_json::from_str(json_str)
            .expect("Flexible deserialization must support numeric booleans and float numbers");
        assert_eq!(patch.email_alerts_enabled, Some(true));
        assert_eq!(patch.motion_enabled, Some(false));
        assert_eq!(patch.smtp_port, Some(587));
        assert_eq!(patch.sensitivity, Some(80));
    }

    #[test]
    fn test_is_authenticated_request_stale_cookie_with_valid_bearer() {
        let auth = AuthManager::new();
        let code = auth.get_pairing_code();
        let valid_token = auth.verify_pairing_code(&code).unwrap();

        let state = AppState {
            config: Arc::new(RwLock::new(SurveillanceConfig::default())),
            auth: auth.clone(),
            broadcaster: FrameBroadcaster::new(),
            audio_broadcaster: AudioBroadcaster::new(),
            db: Arc::new(EventDatabase::new_in_memory().unwrap()),
            ws_sender: broadcast::channel(16).0,
            is_monitoring: Arc::new(AtomicBool::new(false)),
            switch_camera_requested: Arc::new(AtomicBool::new(false)),
            resolution_change_requested: Arc::new(AtomicU8::new(0)),
            test_email_requested: Arc::new(AtomicBool::new(false)),
            device_status: Arc::new(RwLock::new(None)),
        };

        // Header with an invalid/stale cookie AND a valid Bearer token
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static("watchcam_session=revoked_or_invalid_cookie_token"),
        );
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", valid_token)).unwrap(),
        );

        // Crucial test: despite the invalid cookie, the request MUST be authenticated via Bearer token!
        assert!(is_authenticated_request(&state, &headers, None));
    }

    #[test]
    fn test_extract_all_session_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            HeaderValue::from_static("foo=bar; watchcam_session=token_from_cookie; baz=qux"),
        );
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer token_from_bearer"),
        );

        let tokens = extract_all_session_tokens(&headers);
        assert_eq!(tokens.len(), 2);
        assert!(tokens.contains(&"token_from_bearer".to_string()));
        assert!(tokens.contains(&"token_from_cookie".to_string()));
    }

    #[tokio::test]
    async fn test_handle_event_video_full_and_range_requests() {
        let auth = AuthManager::new();
        let code = auth.get_pairing_code();
        let valid_token = auth.verify_pairing_code(&code).unwrap();
        let db = EventDatabase::new_in_memory().unwrap();

        let video_file = db.storage_dir.join("motion_test123.mp4");
        let dummy_data = b"0123456789ABCDEF0123456789ABCDEF"; // 32 bytes
        fs::write(&video_file, dummy_data).unwrap();

        let ev = MotionEvent {
            id: "test123".to_string(),
            timestamp: 1000,
            motion_score: 0.8,
            camera_id: "back".to_string(),
            snapshot_path: "/tmp/snap_test.jpg".to_string(),
            event_type: "motion".to_string(),
            region: None,
            video_path: Some(video_file.to_string_lossy().to_string()),
            has_video: true,
            entity_type: "motion".to_string(),
            confidence: 0.8,
        };
        db.insert_event(&ev).unwrap();

        let state = AppState {
            config: Arc::new(RwLock::new(SurveillanceConfig::default())),
            auth,
            broadcaster: FrameBroadcaster::new(),
            audio_broadcaster: AudioBroadcaster::new(),
            db: Arc::new(db),
            ws_sender: broadcast::channel(16).0,
            is_monitoring: Arc::new(AtomicBool::new(false)),
            switch_camera_requested: Arc::new(AtomicBool::new(false)),
            resolution_change_requested: Arc::new(AtomicU8::new(0)),
            test_email_requested: Arc::new(AtomicBool::new(false)),
            device_status: Arc::new(RwLock::new(None)),
        };

        // 1. Full content request
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", valid_token)).unwrap(),
        );

        let resp = handle_event_video(
            State(state.clone()),
            AxumPath("test123".to_string()),
            headers.clone(),
            Query(WsAuthQuery::default()),
        ).await.unwrap().into_response();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(axum::http::header::CONTENT_TYPE).unwrap(), "video/mp4");
        assert_eq!(resp.headers().get(axum::http::header::ACCEPT_RANGES).unwrap(), "bytes");
        assert_eq!(resp.headers().get(axum::http::header::CONTENT_LENGTH).unwrap(), "32");

        // 2. Partial Range request (bytes 5-14 -> 10 bytes)
        headers.insert(axum::http::header::RANGE, HeaderValue::from_static("bytes=5-14"));
        let range_resp = handle_event_video(
            State(state),
            AxumPath("test123".to_string()),
            headers,
            Query(WsAuthQuery::default()),
        ).await.unwrap().into_response();

        assert_eq!(range_resp.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(range_resp.headers().get(axum::http::header::CONTENT_LENGTH).unwrap(), "10");
        assert_eq!(range_resp.headers().get(axum::http::header::CONTENT_RANGE).unwrap(), "bytes 5-14/32");
    }

    #[tokio::test]
    async fn test_handle_wav_stream_unauthorized_and_authorized() {
        let auth = AuthManager::new();
        let code = auth.get_pairing_code();
        let valid_token = auth.verify_pairing_code(&code).unwrap();

        let state = AppState {
            config: Arc::new(RwLock::new(SurveillanceConfig::default())),
            auth: auth.clone(),
            broadcaster: FrameBroadcaster::new(),
            audio_broadcaster: AudioBroadcaster::new(),
            db: Arc::new(EventDatabase::new_in_memory().unwrap()),
            ws_sender: broadcast::channel(16).0,
            is_monitoring: Arc::new(AtomicBool::new(true)),
            switch_camera_requested: Arc::new(AtomicBool::new(false)),
            resolution_change_requested: Arc::new(AtomicU8::new(0)),
            test_email_requested: Arc::new(AtomicBool::new(false)),
            device_status: Arc::new(RwLock::new(None)),
        };

        // Unauthorized request
        let unauth_res = handle_wav_stream(
            State(state.clone()),
            HeaderMap::new(),
            Query(WsAuthQuery::default()),
        ).await;
        assert_eq!(unauth_res.err(), Some(StatusCode::UNAUTHORIZED));

        // Authorized request
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", valid_token)).unwrap(),
        );

        let auth_res = handle_wav_stream(
            State(state),
            headers,
            Query(WsAuthQuery::default()),
        ).await;
        assert!(auth_res.is_ok());
        let resp = auth_res.unwrap();
        assert_eq!(resp.headers().get(axum::http::header::CONTENT_TYPE).unwrap(), "audio/wav");
    }

    #[test]
    fn test_client_audio_dispatch_callback() {
        use std::sync::atomic::AtomicBool;
        let received = Arc::new(AtomicBool::new(false));
        let r_clone = received.clone();

        crate::jni_api::set_client_audio_callback(Some(move |data: &[u8]| {
            if data == &[1, 2, 3, 4] {
                r_clone.store(true, Ordering::Relaxed);
            }
        }));

        crate::jni_api::dispatch_client_audio(&[1, 2, 3, 4]);
        assert!(received.load(Ordering::Relaxed));

        crate::jni_api::set_client_audio_callback::<fn(&[u8])>(None);
    }

    #[tokio::test]
    async fn test_handle_status_includes_audio_telemetry() {
        let auth = AuthManager::new();
        let broadcaster = FrameBroadcaster::new();
        let audio_broadcaster = AudioBroadcaster::new();
        audio_broadcaster.broadcast_audio(&[1, 2, 3, 4], 75);
        audio_broadcaster.increment_viewer();

        let state = AppState {
            config: Arc::new(RwLock::new(SurveillanceConfig::default())),
            device_status: Arc::new(RwLock::new(None)),
            auth,
            broadcaster,
            audio_broadcaster,
            db: Arc::new(EventDatabase::new_in_memory().unwrap()),
            ws_sender: broadcast::channel(16).0,
            is_monitoring: Arc::new(AtomicBool::new(false)),
            switch_camera_requested: Arc::new(AtomicBool::new(false)),
            resolution_change_requested: Arc::new(AtomicU8::new(0)),
            test_email_requested: Arc::new(AtomicBool::new(false)),
        };

        let resp = handle_status(
            State(state),
            HeaderMap::new(),
            Query(WsAuthQuery::default()),
        ).await;

        let json_resp = resp.0;
        assert_eq!(json_resp["audioLevel"], 75);
        assert_eq!(json_resp["audioViewerCount"], 1);
        assert_eq!(json_resp["serverPort"], 8080);
        assert_eq!(json_resp["httpsPort"], 8443);
    }
}
