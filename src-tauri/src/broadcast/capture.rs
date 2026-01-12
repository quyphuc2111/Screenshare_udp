use scrap::{Capturer, Display};
use std::io::ErrorKind;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use std::sync::Arc;

use super::types::BroadcastError;

pub struct ScreenCapture {
    capturer: Arc<Mutex<Option<Capturer>>>,
    width: u32,
    height: u32,
    last_capture: Instant,
    frame_interval: Duration,
}

impl ScreenCapture {
    pub fn new(fps: u32) -> Result<Self, BroadcastError> {
        let display = Display::primary()
            .map_err(|e| BroadcastError::CaptureError(format!("No primary display: {}", e)))?;
        
        let width = display.width() as u32;
        let height = display.height() as u32;
        
        let capturer = Capturer::new(display)
            .map_err(|e| BroadcastError::CaptureError(format!("Failed to create capturer: {}", e)))?;
        
        Ok(Self {
            capturer: Arc::new(Mutex::new(Some(capturer))),
            width,
            height,
            last_capture: Instant::now(),
            frame_interval: Duration::from_millis(1000 / fps as u64),
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Capture a frame and return RGB data
    pub fn capture_frame(&mut self) -> Result<Option<Vec<u8>>, BroadcastError> {
        // Rate limiting
        let elapsed = self.last_capture.elapsed();
        if elapsed < self.frame_interval {
            return Ok(None);
        }
        
        let mut capturer_guard = self.capturer.lock();
        let capturer = capturer_guard.as_mut()
            .ok_or_else(|| BroadcastError::CaptureError("Capturer not initialized".into()))?;
        
        match capturer.frame() {
            Ok(frame) => {
                self.last_capture = Instant::now();
                // Convert from BGRA to RGB for encoder
                let rgb_data = bgra_to_rgb(&frame, self.width as usize, self.height as usize);
                Ok(Some(rgb_data))
            }
            Err(e) => {
                // Check if it's a WouldBlock error
                if e.kind() == ErrorKind::WouldBlock {
                    // No new frame available
                    Ok(None)
                } else {
                    Err(BroadcastError::CaptureError(format!("Capture failed: {}", e)))
                }
            }
        }
    }

    pub fn set_fps(&mut self, fps: u32) {
        self.frame_interval = Duration::from_millis(1000 / fps.max(1) as u64);
    }
}

/// Convert BGRA to RGB
fn bgra_to_rgb(bgra: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(width * height * 3);
    let stride = bgra.len() / height;
    
    for y in 0..height {
        for x in 0..width {
            let idx = y * stride + x * 4;
            if idx + 2 < bgra.len() {
                rgb.push(bgra[idx + 2]); // R
                rgb.push(bgra[idx + 1]); // G
                rgb.push(bgra[idx]);     // B
            }
        }
    }
    rgb
}

/// Convert RGB to YUV I420 (planar format for H.264)
pub fn rgb_to_yuv420(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let y_size = width * height;
    let uv_size = y_size / 4;
    let mut yuv = vec![0u8; y_size + uv_size * 2];
    
    let (y_plane, uv_planes) = yuv.split_at_mut(y_size);
    let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);
    
    for j in 0..height {
        for i in 0..width {
            let rgb_idx = (j * width + i) * 3;
            if rgb_idx + 2 >= rgb.len() {
                continue;
            }
            let r = rgb[rgb_idx] as i32;
            let g = rgb[rgb_idx + 1] as i32;
            let b = rgb[rgb_idx + 2] as i32;
            
            // BT.601 conversion
            let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            y_plane[j * width + i] = y.clamp(0, 255) as u8;
            
            // Subsample U and V (2x2 blocks)
            if j % 2 == 0 && i % 2 == 0 {
                let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                let uv_idx = (j / 2) * (width / 2) + (i / 2);
                if uv_idx < u_plane.len() {
                    u_plane[uv_idx] = u.clamp(0, 255) as u8;
                    v_plane[uv_idx] = v.clamp(0, 255) as u8;
                }
            }
        }
    }
    
    yuv
}
