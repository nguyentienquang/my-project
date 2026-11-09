use crate::WatchCamEngine;
use jni::objects::{GlobalRef, JByteArray, JByteBuffer, JClass, JString};
use jni::sys::{jboolean, jfloatArray, jint, jstring, JNI_FALSE, JNI_TRUE};
use jni::{JNIEnv, JavaVM};

static JAVA_VM: parking_lot::RwLock<Option<JavaVM>> = parking_lot::RwLock::new(None);
static NATIVE_CLASS: parking_lot::RwLock<Option<GlobalRef>> = parking_lot::RwLock::new(None);
static CLIENT_AUDIO_CB: parking_lot::RwLock<Option<Box<dyn Fn(&[u8]) + Send + Sync + 'static>>> = parking_lot::RwLock::new(None);

pub fn set_client_audio_callback<F: Fn(&[u8]) + Send + Sync + 'static>(cb: Option<F>) {
    *CLIENT_AUDIO_CB.write() = cb.map(|f| Box::new(f) as _);
}

pub fn dispatch_client_audio(data: &[u8]) {
    if let Some(cb) = CLIENT_AUDIO_CB.read().as_ref() {
        cb(data);
    }

    let vm_guard = JAVA_VM.read();
    if let Some(vm) = vm_guard.as_ref() {
        // Permanently attach thread to avoid JVM thread allocate/detach churn on every audio chunk
        let mut env = match vm.attach_current_thread_permanently() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("dispatch_client_audio attach thread error: {:?}", e);
                return;
            }
        };

        let g_class = {
            let class_guard = NATIVE_CLASS.read();
            if let Some(c) = class_guard.as_ref() {
                Some(c.clone())
            } else {
                drop(class_guard);
                if let Ok(cls) = env.find_class("com/watchcam/android/nativebridge/WatchCamNative") {
                    if let Ok(gref) = env.new_global_ref(&cls) {
                        *NATIVE_CLASS.write() = Some(gref.clone());
                        Some(gref)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        };

        if let Some(gref) = g_class {
            if let Ok(byte_array) = env.byte_array_from_slice(data) {
                let class = unsafe { jni::objects::JClass::from_raw(gref.as_obj().as_raw()) };
                let res = env.call_static_method(
                    &class,
                    "receiveClientAudio",
                    "([B)V",
                    &[jni::objects::JValue::Object(&byte_array)],
                );
                if let Err(e) = res {
                    eprintln!("dispatch_client_audio call_static_method error: {:?}", e);
                    if let Ok(has_ex) = env.exception_check() {
                        if has_ex {
                            let _ = env.exception_clear();
                        }
                    }
                }
            }
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_initServer(
    mut env: JNIEnv,
    _class: JClass,
    storage_dir: JString,
    db_path: JString,
    port: jint,
) -> jboolean {
    if let Ok(vm) = env.get_java_vm() {
        *JAVA_VM.write() = Some(vm);
    }
    match env.find_class("com/watchcam/android/nativebridge/WatchCamNative") {
        Ok(cls) => {
            if let Ok(gref) = env.new_global_ref(&cls) {
                *NATIVE_CLASS.write() = Some(gref);
            }
        }
        Err(e) => {
            eprintln!("Failed to find WatchCamNative class: {:?}", e);
        }
    }

    let storage_dir_str: String = match env.get_string(&storage_dir) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    let db_path_str: String = match env.get_string(&db_path) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    match WatchCamEngine::initialize(storage_dir_str, db_path_str, port as u16) {
        Ok(_) => JNI_TRUE,
        Err(e) => {
            eprintln!("WatchCamEngine initialize failed: {}", e);
            JNI_FALSE
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_stopServer(
    _env: JNIEnv,
    _class: JClass,
) {
    WatchCamEngine::stop();
    *JAVA_VM.write() = None;
    *NATIVE_CLASS.write() = None;
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_pushAudioFrame(
    env: JNIEnv,
    _class: JClass,
    data: JByteArray,
    length: jint,
    level: jint,
) {
    if length <= 0 {
        return;
    }
    if let Ok(bytes) = env.convert_byte_array(&data) {
        let len = (length as usize).min(bytes.len());
        WatchCamEngine::push_audio_frame(&bytes[..len], level.clamp(0, 100) as u8);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_processFrameYuv(
    env: JNIEnv,
    _class: JClass,
    y_buffer: JByteBuffer,
    u_buffer: JByteBuffer,
    v_buffer: JByteBuffer,
    width: jint,
    height: jint,
    y_row_stride: jint,
    u_row_stride: jint,
    v_row_stride: jint,
    u_pixel_stride: jint,
    v_pixel_stride: jint,
    rotation: jint,
) -> jfloatArray {
    let empty_arr = match env.new_float_array(12) {
        Ok(arr) => arr,
        Err(_) => return std::ptr::null_mut(),
    };

    let y_ptr = match env.get_direct_buffer_address(&y_buffer) {
        Ok(p) => p,
        Err(_) => return empty_arr.into_raw(),
    };
    let y_len = match env.get_direct_buffer_capacity(&y_buffer) {
        Ok(len) => len,
        Err(_) => return empty_arr.into_raw(),
    };

    if y_ptr.is_null() || y_len == 0 {
        return empty_arr.into_raw();
    }

    let u_ptr = env.get_direct_buffer_address(&u_buffer).unwrap_or(std::ptr::null_mut());
    let u_len = env.get_direct_buffer_capacity(&u_buffer).unwrap_or(0);

    let v_ptr = env.get_direct_buffer_address(&v_buffer).unwrap_or(std::ptr::null_mut());
    let v_len = env.get_direct_buffer_capacity(&v_buffer).unwrap_or(0);

    let y_slice = unsafe { std::slice::from_raw_parts(y_ptr, y_len) };
    let u_slice = if !u_ptr.is_null() && u_len > 0 {
        unsafe { std::slice::from_raw_parts(u_ptr, u_len) }
    } else {
        &[]
    };
    let v_slice = if !v_ptr.is_null() && v_len > 0 {
        unsafe { std::slice::from_raw_parts(v_ptr, v_len) }
    } else {
        &[]
    };

    let (
        alert_triggered,
        motion_score,
        should_switch_cam,
        is_monitoring,
        res_change,
        test_email_req,
        entity_code,
        confidence,
        bx,
        by,
        bw,
        bh,
    ) = WatchCamEngine::process_camera_frame_yuv(
        y_slice,
        u_slice,
        v_slice,
        width as usize,
        height as usize,
        y_row_stride as usize,
        u_row_stride as usize,
        v_row_stride as usize,
        u_pixel_stride as usize,
        v_pixel_stride as usize,
        rotation,
    );

    let results = [
        if alert_triggered { 1.0f32 } else { 0.0f32 },
        motion_score,
        if should_switch_cam { 1.0f32 } else { 0.0f32 },
        if is_monitoring { 1.0f32 } else { 0.0f32 },
        res_change,
        if test_email_req { 1.0f32 } else { 0.0f32 },
        entity_code,
        confidence,
        bx,
        by,
        bw,
        bh,
    ];

    let _ = env.set_float_array_region(&empty_arr, 0, &results);
    empty_arr.into_raw()
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_processFrame(
    env: JNIEnv,
    _class: JClass,
    buffer: JByteBuffer,
    width: jint,
    height: jint,
    row_stride: jint,
    rotation: jint,
) -> jfloatArray {
    let empty_arr = match env.new_float_array(12) {
        Ok(arr) => arr,
        Err(_) => return std::ptr::null_mut(),
    };

    let buf_ptr = match env.get_direct_buffer_address(&buffer) {
        Ok(p) => p,
        Err(_) => return empty_arr.into_raw(),
    };

    let buf_len = match env.get_direct_buffer_capacity(&buffer) {
        Ok(len) => len,
        Err(_) => return empty_arr.into_raw(),
    };

    if buf_ptr.is_null() || buf_len == 0 {
        return empty_arr.into_raw();
    }

    let slice = unsafe { std::slice::from_raw_parts(buf_ptr, buf_len) };

    let (
        alert_triggered,
        motion_score,
        should_switch_cam,
        is_monitoring,
        res_change,
        test_email_req,
        entity_code,
        confidence,
        bx,
        by,
        bw,
        bh,
    ) = WatchCamEngine::process_camera_frame_yuv(
        slice,
        &[],
        &[],
        width as usize,
        height as usize,
        row_stride as usize,
        0,
        0,
        0,
        0,
        rotation,
    );

    let results = [
        if alert_triggered { 1.0f32 } else { 0.0f32 },
        motion_score,
        if should_switch_cam { 1.0f32 } else { 0.0f32 },
        if is_monitoring { 1.0f32 } else { 0.0f32 },
        res_change,
        if test_email_req { 1.0f32 } else { 0.0f32 },
        entity_code,
        confidence,
        bx,
        by,
        bw,
        bh,
    ];

    let _ = env.set_float_array_region(&empty_arr, 0, &results);
    empty_arr.into_raw()
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_getPairingCode(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let code = WatchCamEngine::get_pairing_code();
    match env.new_string(code) {
        Ok(js) => js.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_regeneratePairingCode(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let code = WatchCamEngine::regenerate_pairing_code();
    match env.new_string(code) {
        Ok(js) => js.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_revokeAllSessions(
    _env: JNIEnv,
    _class: JClass,
) {
    WatchCamEngine::revoke_all_sessions();
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_revokeAllSessionsWithDb(
    mut env: JNIEnv,
    _class: JClass,
    db_path: JString,
) {
    let path_str: Result<String, _> = env.get_string(&db_path).map(|js| js.into());
    if let Ok(path) = path_str {
        if !path.is_empty() {
            WatchCamEngine::revoke_all_sessions_with_db(Some(std::path::Path::new(&path)));
            return;
        }
    }
    WatchCamEngine::revoke_all_sessions();
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_updateConfig(
    _env: JNIEnv,
    _class: JClass,
    motion_enabled: jboolean,
    sensitivity: jint,
    stream_quality: jint,
) {
    WatchCamEngine::update_config(
        motion_enabled == JNI_TRUE,
        sensitivity.clamp(0, 100) as u8,
        stream_quality.clamp(10, 100) as u8,
    );
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_updateEmailConfig(
    mut env: JNIEnv,
    _class: JClass,
    enabled: jboolean,
    host: JString,
    port: jint,
    username: JString,
    password: JString,
    recipient: JString,
) {
    let host_str: String = match env.get_string(&host) {
        Ok(s) => s.into(),
        Err(_) => String::new(),
    };
    let username_str: String = match env.get_string(&username) {
        Ok(s) => s.into(),
        Err(_) => String::new(),
    };
    let password_str: String = match env.get_string(&password) {
        Ok(s) => s.into(),
        Err(_) => String::new(),
    };
    let recipient_str: String = match env.get_string(&recipient) {
        Ok(s) => s.into(),
        Err(_) => String::new(),
    };

    WatchCamEngine::update_email_config(
        enabled == JNI_TRUE,
        &host_str,
        port.clamp(1, 65535) as u16,
        &username_str,
        &password_str,
        &recipient_str,
    );
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_setCameraId(
    _env: JNIEnv,
    _class: JClass,
    is_front: jboolean,
) {
    WatchCamEngine::set_camera_id(is_front == JNI_TRUE);
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_setMonitoring(
    _env: JNIEnv,
    _class: JClass,
    enabled: jboolean,
) {
    WatchCamEngine::set_monitoring(enabled == JNI_TRUE);
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_isMonitoring(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    if WatchCamEngine::is_monitoring() {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_getStatusJson(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let status = WatchCamEngine::get_status_json();
    match env.new_string(status) {
        Ok(js) => js.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_getLatestEventJson(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let event = WatchCamEngine::get_latest_event_json();
    match env.new_string(event) {
        Ok(js) => js.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_updateDeviceStatus(
    mut env: JNIEnv,
    _class: JClass,
    status_json: JString,
) -> jboolean {
    let json_str: String = match env.get_string(&status_json) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    if WatchCamEngine::update_device_status(&json_str) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_getDeviceStatusJson(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let status = WatchCamEngine::get_device_status_json();
    match env.new_string(status) {
        Ok(js) => js.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_watchcam_android_nativebridge_WatchCamNative_notifyVideoRecorded(
    mut env: JNIEnv,
    _class: JClass,
    event_id: JString,
    video_path: JString,
) -> jboolean {
    let event_id_str: String = match env.get_string(&event_id) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };
    let video_path_str: String = match env.get_string(&video_path) {
        Ok(s) => s.into(),
        Err(_) => return JNI_FALSE,
    };

    if WatchCamEngine::notify_video_recorded(&event_id_str, &video_path_str) {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}
