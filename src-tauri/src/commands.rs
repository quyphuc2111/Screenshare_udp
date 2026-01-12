use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use once_cell::sync::Lazy;
use tauri::{AppHandle, Emitter};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::broadcast::{
    BroadcastConfig, BroadcastStats, BroadcastError,
    ScreenCapture, H264Encoder, MulticastSender, StreamReceiver,
};

// Global state for teacher broadcasting
static TEACHER_STATE: Lazy<Arc<Mutex<Option<TeacherBroadcaster>>>> = 
    Lazy::new(|| Arc::new(Mutex::new(None)));

// Global state for student receiving
static STUDENT_STATE: Lazy<Arc<Mutex<Option<StudentReceiver>>>> = 
    Lazy::new(|| Arc::new(Mutex::new(None)));

// Log buffer for UI
static LOG_BUFFER: Lazy<Arc<Mutex<Vec<String>>>> = 
    Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

fn add_log(msg: &str) {
    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
    let log_msg = format!("[{}] {}", timestamp, msg);
    log::info!("{}", msg);
    let mut buffer = LOG_BUFFER.lock();
    buffer.push(log_msg);
    // Keep last 100 logs
    if buffer.len() > 100 {
        buffer.remove(0);
    }
}

#[tauri::command]
pub fn get_logs() -> Vec<String> {
    LOG_BUFFER.lock().clone()
}

#[tauri::command]
pub fn clear_logs() {
    LOG_BUFFER.lock().clear();
}

struct TeacherBroadcaster {
    running: Arc<Mutex<bool>>,
    stats: Arc<Mutex<BroadcastStats>>,
    config: BroadcastConfig,
}

struct StudentReceiver {
    running: Arc<Mutex<bool>>,
    config: BroadcastConfig,
}

#[tauri::command]
pub fn get_default_config() -> BroadcastConfig {
    BroadcastConfig::default()
}

#[tauri::command]
pub async fn start_teacher_broadcast(
    app: AppHandle,
    config: BroadcastConfig,
) -> Result<(), String> {
    let mut state = TEACHER_STATE.lock();
    
    if state.is_some() {
        return Err("Broadcast already running".into());
    }

    let running = Arc::new(Mutex::new(true));
    let stats = Arc::new(Mutex::new(BroadcastStats {
        fps: 0.0,
        bitrate_kbps: 0.0,
        frame_count: 0,
        dropped_frames: 0,
        cpu_usage: 0.0,
        latency_ms: 0.0,
    }));

    let broadcaster = TeacherBroadcaster {
        running: running.clone(),
        stats: stats.clone(),
        config: config.clone(),
    };

    *state = Some(broadcaster);
    drop(state);

    // Start broadcast thread
    let running_clone = running.clone();
    let stats_clone = stats.clone();
    let app_clone = app.clone();
    
    thread::spawn(move || {
        if let Err(e) = run_teacher_broadcast(running_clone, stats_clone, config, app_clone) {
            log::error!("Broadcast error: {}", e);
        }
    });

    Ok(())
}

fn run_teacher_broadcast(
    running: Arc<Mutex<bool>>,
    stats: Arc<Mutex<BroadcastStats>>,
    config: BroadcastConfig,
    app: AppHandle,
) -> Result<(), BroadcastError> {
    add_log(&format!("Initializing capture with {} fps", config.fps));
    
    // Initialize components
    let mut capture = ScreenCapture::new(config.fps)?;
    let (width, height) = capture.dimensions();
    
    add_log(&format!("Screen size: {}x{}", width, height));
    
    let bitrate = calculate_bitrate(width, height, config.fps, config.quality);
    add_log(&format!("Creating H264 encoder, bitrate: {} kbps", bitrate));
    
    let mut encoder = H264Encoder::new(width, height, config.fps, bitrate)?;
    
    add_log(&format!("Creating multicast sender: {}:{}", config.multicast_addr, config.port));
    let mut sender = MulticastSender::new(&config.multicast_addr, config.port)?;

    let frame_interval = Duration::from_millis(1000 / config.fps as u64);
    let mut last_stats_update = Instant::now();
    let mut frames_since_stats = 0u32;
    let mut bytes_since_stats = 0u64;
    let mut dropped = 0u64;

    add_log(&format!("Teacher broadcast started: {}x{} @ {} fps, {} kbps", 
               width, height, config.fps, bitrate));

    while *running.lock() {
        let frame_start = Instant::now();

        // Capture screen
        match capture.capture_frame() {
            Ok(Some(rgb_data)) => {
                // Encode to H.264
                match encoder.encode(&rgb_data) {
                    Ok((h264_data, is_keyframe)) => {
                        bytes_since_stats += h264_data.len() as u64;
                        
                        // Send via multicast
                        if let Err(e) = sender.send_frame(&h264_data, is_keyframe) {
                            add_log(&format!("Send error: {}", e));
                            dropped += 1;
                        }
                        
                        frames_since_stats += 1;
                    }
                    Err(e) => {
                        add_log(&format!("Encode error: {}", e));
                        dropped += 1;
                    }
                }
            }
            Ok(None) => {
                // No new frame, sleep briefly
                thread::sleep(Duration::from_millis(1));
            }
            Err(e) => {
                add_log(&format!("Capture error: {}", e));
                dropped += 1;
            }
        }

        // Update stats every second
        if last_stats_update.elapsed() >= Duration::from_secs(1) {
            let elapsed = last_stats_update.elapsed().as_secs_f32();
            let mut s = stats.lock();
            s.fps = frames_since_stats as f32 / elapsed;
            s.bitrate_kbps = (bytes_since_stats as f32 * 8.0 / 1000.0) / elapsed;
            s.frame_count += frames_since_stats as u64;
            s.dropped_frames = dropped;
            s.latency_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
            
            // Emit stats to frontend
            let _ = app.emit("broadcast-stats", s.clone());
            
            frames_since_stats = 0;
            bytes_since_stats = 0;
            last_stats_update = Instant::now();
        }

        // Frame rate limiting
        let elapsed = frame_start.elapsed();
        if elapsed < frame_interval {
            thread::sleep(frame_interval - elapsed);
        }
    }

    add_log("Teacher broadcast stopped");
    Ok(())
}

#[tauri::command]
pub fn stop_teacher_broadcast() -> Result<(), String> {
    let mut state = TEACHER_STATE.lock();
    
    if let Some(broadcaster) = state.take() {
        *broadcaster.running.lock() = false;
        Ok(())
    } else {
        Err("No broadcast running".into())
    }
}

#[tauri::command]
pub fn get_teacher_stats() -> Option<BroadcastStats> {
    let state = TEACHER_STATE.lock();
    state.as_ref().map(|b| b.stats.lock().clone())
}

#[tauri::command]
pub async fn start_student_receiver(
    app: AppHandle,
    config: BroadcastConfig,
) -> Result<(), String> {
    let mut state = STUDENT_STATE.lock();
    
    if state.is_some() {
        return Err("Receiver already running".into());
    }

    let running = Arc::new(Mutex::new(true));

    let receiver_state = StudentReceiver {
        running: running.clone(),
        config: config.clone(),
    };

    *state = Some(receiver_state);
    drop(state);

    // Start receiver thread
    let running_clone = running.clone();
    let app_clone = app.clone();
    
    thread::spawn(move || {
        if let Err(e) = run_student_receiver(running_clone, config, app_clone) {
            log::error!("Receiver error: {}", e);
        }
    });

    Ok(())
}

fn run_student_receiver(
    running: Arc<Mutex<bool>>,
    config: BroadcastConfig,
    app: AppHandle,
) -> Result<(), BroadcastError> {
    add_log(&format!("Creating receiver for {}:{}", config.multicast_addr, config.port));
    
    let mut receiver = StreamReceiver::new(&config)?;
    
    add_log(&format!("Student receiver started, listening on {}:{}", 
               config.multicast_addr, config.port));

    let mut last_frame_time = Instant::now();
    let mut frame_count = 0u64;
    let mut last_log_time = Instant::now();
    let mut packets_received = 0u64;

    while *running.lock() {
        match receiver.process() {
            Ok(Some(frame)) => {
                frame_count += 1;
                packets_received += 1;
                
                // Encode frame as base64 for frontend
                let frame_data = FrameData {
                    width: frame.width,
                    height: frame.height,
                    data: BASE64.encode(&frame.rgba_data),
                    timestamp: frame.timestamp,
                    is_keyframe: frame.is_keyframe,
                };
                
                // Emit frame to frontend
                let _ = app.emit("video-frame", frame_data);
                
                // Log FPS periodically
                if frame_count % 30 == 0 {
                    let fps = 30.0 / last_frame_time.elapsed().as_secs_f32();
                    add_log(&format!("Receiving at {:.1} fps, frames: {}", fps, frame_count));
                    last_frame_time = Instant::now();
                }
            }
            Ok(None) => {
                // No frame ready, brief sleep
                thread::sleep(Duration::from_millis(1));
                
                // Log status every 5 seconds if no frames
                if last_log_time.elapsed() >= Duration::from_secs(5) {
                    if packets_received == 0 {
                        add_log("No packets received yet... Check firewall and network");
                    }
                    last_log_time = Instant::now();
                }
            }
            Err(e) => {
                add_log(&format!("Receive error: {}", e));
            }
        }
    }

    add_log("Student receiver stopped");
    Ok(())
}

#[tauri::command]
pub fn stop_student_receiver() -> Result<(), String> {
    let mut state = STUDENT_STATE.lock();
    
    if let Some(receiver) = state.take() {
        *receiver.running.lock() = false;
        Ok(())
    } else {
        Err("No receiver running".into())
    }
}

#[tauri::command]
pub fn is_teacher_broadcasting() -> bool {
    TEACHER_STATE.lock().is_some()
}

#[tauri::command]
pub fn is_student_receiving() -> bool {
    STUDENT_STATE.lock().is_some()
}

#[derive(Clone, serde::Serialize)]
struct FrameData {
    width: u32,
    height: u32,
    data: String, // Base64 encoded RGBA
    timestamp: u32,
    is_keyframe: bool,
}

/// Calculate appropriate bitrate based on resolution and quality
fn calculate_bitrate(width: u32, height: u32, fps: u32, quality: u32) -> u32 {
    let pixels = width * height;
    let base_bitrate = match pixels {
        p if p <= 921600 => 1500,   // 720p: 1.5 Mbps
        p if p <= 2073600 => 3000,  // 1080p: 3 Mbps
        _ => 5000,                   // 4K: 5 Mbps
    };
    
    // Adjust for FPS (base is 30fps)
    let fps_factor = fps as f32 / 30.0;
    
    // Adjust for quality (lower quality = lower bitrate)
    let quality_factor = 1.0 - (quality as f32 - 20.0) / 60.0;
    
    (base_bitrate as f32 * fps_factor * quality_factor.max(0.3)) as u32
}
