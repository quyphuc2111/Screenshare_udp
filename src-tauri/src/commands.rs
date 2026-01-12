use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use once_cell::sync::Lazy;
use tauri::{AppHandle, Emitter};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

use crate::broadcast::{
    StreamConfig, StreamStats, BroadcastError,
    ScreenCapture, H264Encoder, H264Decoder,
    RtpSender, RtpReceiver,
    DiscoveryService, PeerInfo, PeerRole,
};

// Global state
static TEACHER_RUNNING: Lazy<Arc<Mutex<bool>>> = Lazy::new(|| Arc::new(Mutex::new(false)));
static STUDENT_RUNNING: Lazy<Arc<Mutex<bool>>> = Lazy::new(|| Arc::new(Mutex::new(false)));
static DISCOVERY: Lazy<Arc<Mutex<Option<DiscoveryService>>>> = Lazy::new(|| Arc::new(Mutex::new(None)));
static LOGS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

fn log_msg(msg: &str) {
    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
    let log_entry = format!("[{}] {}", timestamp, msg);
    log::info!("{}", msg);
    
    let mut logs = LOGS.lock();
    logs.push(log_entry);
    if logs.len() > 100 {
        logs.remove(0);
    }
}

// ============ Config Commands ============

#[tauri::command]
pub fn get_default_config() -> StreamConfig {
    StreamConfig::default()
}

#[tauri::command]
pub fn get_logs() -> Vec<String> {
    LOGS.lock().clone()
}

#[tauri::command]
pub fn clear_logs() {
    LOGS.lock().clear();
}

// ============ Discovery Commands ============

#[tauri::command]
pub fn start_discovery(name: String, is_teacher: bool, port: u16) -> Result<(), String> {
    let role = if is_teacher { PeerRole::Teacher } else { PeerRole::Student };
    
    let service = DiscoveryService::new(&name, role, port)
        .map_err(|e| format!("Failed to start discovery: {}", e))?;
    
    service.start().map_err(|e| e.to_string())?;
    
    *DISCOVERY.lock() = Some(service);
    log_msg(&format!("Discovery started as {:?}: {}", role, name));
    
    Ok(())
}

#[tauri::command]
pub fn stop_discovery() {
    if let Some(service) = DISCOVERY.lock().take() {
        service.stop();
        log_msg("Discovery stopped");
    }
}

#[tauri::command]
pub fn discovery_announce() -> Result<(), String> {
    if let Some(ref service) = *DISCOVERY.lock() {
        service.announce().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn discovery_query() -> Result<(), String> {
    if let Some(ref service) = *DISCOVERY.lock() {
        service.query().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_discovered_peers() -> Vec<PeerInfo> {
    if let Some(ref service) = *DISCOVERY.lock() {
        // Process any pending messages
        while let Ok(Some(peer)) = service.process() {
            log_msg(&format!("Discovered: {} ({:?}) at {}", peer.name, peer.role, peer.ip));
        }
        return service.get_peers();
    }
    Vec::new()
}

#[tauri::command]
pub fn get_teachers() -> Vec<PeerInfo> {
    if let Some(ref service) = *DISCOVERY.lock() {
        while let Ok(Some(_)) = service.process() {}
        return service.get_teachers();
    }
    Vec::new()
}

// ============ Teacher Commands ============

#[tauri::command]
pub async fn start_teacher(app: AppHandle, config: StreamConfig) -> Result<(), String> {
    if *TEACHER_RUNNING.lock() {
        return Err("Already broadcasting".into());
    }
    
    *TEACHER_RUNNING.lock() = true;
    
    let running = TEACHER_RUNNING.clone();
    
    thread::spawn(move || {
        if let Err(e) = run_teacher(running, config, app) {
            log_msg(&format!("Teacher error: {}", e));
        }
    });
    
    Ok(())
}

fn run_teacher(running: Arc<Mutex<bool>>, config: StreamConfig, app: AppHandle) -> Result<(), BroadcastError> {
    log_msg(&format!("Starting teacher: {:?} mode, port {}, {} fps", 
        config.network_mode, config.port, config.fps));
    
    // Initialize capture
    log_msg("Initializing screen capture...");
    let mut capture = ScreenCapture::new(config.fps)?;
    let (width, height) = capture.dimensions();
    log_msg(&format!("Screen: {}x{}", width, height));
    
    // Test capture immediately
    log_msg("Testing capture...");
    let mut test_attempts = 0;
    let mut test_success = false;
    while test_attempts < 10 && !test_success {
        match capture.capture_frame() {
            Ok(Some(rgb_data)) => {
                log_msg(&format!("Test capture OK: {} bytes RGB data", rgb_data.len()));
                test_success = true;
            }
            Ok(None) => {
                test_attempts += 1;
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                log_msg(&format!("Test capture failed: {}", e));
                return Err(e);
            }
        }
    }
    
    if !test_success {
        log_msg("Warning: Could not capture test frame after 10 attempts");
    }
    
    // Initialize encoder
    let bitrate = calculate_bitrate(width, height, config.fps, config.quality);
    log_msg(&format!("Initializing encoder: {}x{} @ {} kbps", width, height, bitrate));
    let mut encoder = H264Encoder::new(width, height, config.fps, bitrate)?;
    log_msg(&format!("Encoder ready: {} kbps", bitrate));
    
    // Initialize RTP sender
    log_msg(&format!("Initializing RTP sender: {:?} mode, port {}", config.network_mode, config.port));
    let mut sender = RtpSender::new(config.port, config.network_mode)?;
    log_msg("RTP sender ready");
    
    let frame_interval = Duration::from_millis(1000 / config.fps as u64);
    let mut last_stats = Instant::now();
    let mut frames = 0u64;
    let mut bytes = 0u64;
    let mut capture_errors = 0u64;
    let mut encode_errors = 0u64;
    let mut no_frame_count = 0u64;
    let start_time = Instant::now();
    
    log_msg("Broadcasting started!");
    log_msg(&format!("Frame interval: {:?}", frame_interval));
    
    while *running.lock() {
        let frame_start = Instant::now();
        
        // Capture
        match capture.capture_frame() {
            Ok(Some(rgb_data)) => {
                no_frame_count = 0;
                log::debug!("Captured frame: {} bytes RGB", rgb_data.len());
                
                // Encode
                match encoder.encode(&rgb_data) {
                    Ok((h264_data, is_keyframe)) => {
                        if h264_data.is_empty() {
                            log_msg("Encoder produced empty data!");
                        } else {
                            // Send via RTP
                            let timestamp_ms = start_time.elapsed().as_millis() as u32;
                            match sender.send_frame(&h264_data, timestamp_ms) {
                                Ok(sent) => {
                                    frames += 1;
                                    bytes += sent as u64;
                                    
                                    // Log first few frames
                                    if frames <= 5 {
                                        log_msg(&format!("Sent frame {}: {} bytes H264, {} bytes UDP, keyframe={}", 
                                            frames, h264_data.len(), sent, is_keyframe));
                                    }
                                }
                                Err(e) => {
                                    log_msg(&format!("Send error: {}", e));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        encode_errors += 1;
                        if encode_errors <= 5 {
                            log_msg(&format!("Encode error #{}: {}", encode_errors, e));
                        }
                    }
                }
            }
            Ok(None) => {
                // No frame ready yet - rate limited or WouldBlock
                no_frame_count += 1;
            }
            Err(e) => {
                capture_errors += 1;
                if capture_errors <= 5 {
                    log_msg(&format!("Capture error #{}: {}", capture_errors, e));
                }
            }
        }
        
        // Stats every second
        if last_stats.elapsed() >= Duration::from_secs(1) {
            let elapsed = last_stats.elapsed().as_secs_f32();
            let stats = StreamStats {
                fps: frames as f32 / elapsed,
                bitrate_kbps: (bytes as f32 * 8.0 / 1000.0) / elapsed,
                frame_count: sender.frame_count(),
                packets_sent: 0,
                packets_lost: 0,
                latency_ms: frame_start.elapsed().as_secs_f32() * 1000.0,
            };
            
            let _ = app.emit("stream-stats", &stats);
            
            // Detailed stats logging
            log_msg(&format!("Stats: {} fps, {} kbps, frames={}, no_frame={}, cap_err={}, enc_err={}", 
                stats.fps as u32, stats.bitrate_kbps as u32, frames, no_frame_count, capture_errors, encode_errors));
            
            frames = 0;
            bytes = 0;
            no_frame_count = 0;
            last_stats = Instant::now();
        }
        
        // Frame rate control - small sleep to prevent busy loop
        let elapsed = frame_start.elapsed();
        if elapsed < frame_interval {
            thread::sleep(frame_interval - elapsed);
        }
    }
    
    log_msg("Broadcasting stopped");
    Ok(())
}

#[tauri::command]
pub fn stop_teacher() {
    *TEACHER_RUNNING.lock() = false;
    log_msg("Stopping teacher...");
}

#[tauri::command]
pub fn is_teacher_running() -> bool {
    *TEACHER_RUNNING.lock()
}

// ============ Student Commands ============

#[tauri::command]
pub async fn start_student(app: AppHandle, config: StreamConfig) -> Result<(), String> {
    if *STUDENT_RUNNING.lock() {
        return Err("Already receiving".into());
    }
    
    *STUDENT_RUNNING.lock() = true;
    
    let running = STUDENT_RUNNING.clone();
    
    thread::spawn(move || {
        if let Err(e) = run_student(running, config, app) {
            log_msg(&format!("Student error: {}", e));
        }
    });
    
    Ok(())
}

fn run_student(running: Arc<Mutex<bool>>, config: StreamConfig, app: AppHandle) -> Result<(), BroadcastError> {
    log_msg(&format!("Starting student: {:?} mode, port {}", config.network_mode, config.port));
    
    // Initialize RTP receiver
    let mut receiver = RtpReceiver::new(config.port, config.network_mode)?;
    log_msg("RTP receiver ready");
    
    // Initialize decoder
    let mut decoder = H264Decoder::new()?;
    log_msg("Decoder ready");
    
    let mut last_log = Instant::now();
    let mut frames_received = 0u64;
    let mut waiting_for_keyframe = true;
    
    log_msg("Waiting for stream...");
    
    while *running.lock() {
        match receiver.receive_frame() {
            Ok(Some(h264_frame)) => {
                // Check for keyframe (IDR NAL type = 5)
                let is_keyframe = h264_frame.windows(5).any(|w| {
                    (w[0] == 0 && w[1] == 0 && w[2] == 0 && w[3] == 1 && (w[4] & 0x1F) == 5) ||
                    (w[0] == 0 && w[1] == 0 && w[2] == 1 && (w[3] & 0x1F) == 5)
                });
                
                if waiting_for_keyframe {
                    if is_keyframe {
                        log_msg("Got keyframe, starting decode");
                        waiting_for_keyframe = false;
                    } else {
                        continue;
                    }
                }
                
                // Decode
                match decoder.decode(&h264_frame) {
                    Ok(Some(frame)) => {
                        frames_received += 1;
                        
                        // Send to frontend
                        let frame_data = FrameData {
                            width: frame.width,
                            height: frame.height,
                            data: BASE64.encode(&frame.rgba_data),
                        };
                        
                        if let Err(e) = app.emit("video-frame", &frame_data) {
                            log_msg(&format!("Emit error: {}", e));
                        }
                        
                        if frames_received % 30 == 0 {
                            log_msg(&format!("Received {} frames", frames_received));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log_msg(&format!("Decode error: {}", e));
                        waiting_for_keyframe = true;
                    }
                }
            }
            Ok(None) => {
                // No frame yet
                if last_log.elapsed() >= Duration::from_secs(5) && frames_received == 0 {
                    log_msg("No frames received yet...");
                    last_log = Instant::now();
                }
            }
            Err(e) => {
                log_msg(&format!("Receive error: {}", e));
            }
        }
    }
    
    log_msg(&format!("Receiving stopped. Total frames: {}", frames_received));
    Ok(())
}

#[tauri::command]
pub fn stop_student() {
    *STUDENT_RUNNING.lock() = false;
    log_msg("Stopping student...");
}

#[tauri::command]
pub fn is_student_running() -> bool {
    *STUDENT_RUNNING.lock()
}

// ============ Helpers ============

#[derive(Clone, serde::Serialize)]
struct FrameData {
    width: u32,
    height: u32,
    data: String,
}

fn calculate_bitrate(width: u32, height: u32, fps: u32, quality: u32) -> u32 {
    let pixels = width * height;
    let base = match pixels {
        p if p <= 921600 => 1500,   // 720p
        p if p <= 2073600 => 3000,  // 1080p
        _ => 5000,
    };
    
    let fps_factor = fps as f32 / 30.0;
    let quality_factor = 1.0 - (quality as f32 - 20.0) / 60.0;
    
    (base as f32 * fps_factor * quality_factor.max(0.3)) as u32
}
