use openh264::encoder::{Encoder, EncoderConfig};
use openh264::formats::YUVSource;
use openh264::OpenH264API;

use super::types::BroadcastError;

pub struct H264Encoder {
    encoder: Encoder,
    width: u32,
    height: u32,
    frame_count: u64,
    // Pre-allocated YUV buffer for zero-copy
    yuv_buffer: Vec<u8>,
}

impl H264Encoder {
    pub fn new(width: u32, height: u32, fps: u32, bitrate_kbps: u32) -> Result<Self, BroadcastError> {
        let api = OpenH264API::from_source();
        
        // Optimize for LOW LATENCY
        let config = EncoderConfig::new()
            .set_bitrate_bps(bitrate_kbps * 1000)
            .max_frame_rate(fps as f32)
            .enable_skip_frame(false);
        
        let encoder = Encoder::with_api_config(api, config)
            .map_err(|e| BroadcastError::EncoderError(format!("Failed to create encoder: {}", e)))?;
        
        // Pre-allocate YUV buffer
        let y_size = (width * height) as usize;
        let uv_size = y_size / 4;
        let yuv_buffer = vec![0u8; y_size + uv_size * 2];
        
        log::info!("H264 Encoder: {}x{} @ {} fps, {} kbps", width, height, fps, bitrate_kbps);
        
        Ok(Self {
            encoder,
            width,
            height,
            frame_count: 0,
            yuv_buffer,
        })
    }

    /// Encode RGB frame to H.264 - OPTIMIZED for low latency
    #[inline]
    pub fn encode(&mut self, rgb_data: &[u8]) -> Result<(Vec<u8>, bool), BroadcastError> {
        // Fast RGB to YUV conversion (in-place)
        self.rgb_to_yuv420_fast(rgb_data);
        
        let yuv_source = YUVBufferRef {
            data: &self.yuv_buffer,
            width: self.width as usize,
            height: self.height as usize,
        };
        
        // Encode
        let bitstream = self.encoder.encode(&yuv_source)
            .map_err(|e| BroadcastError::EncoderError(format!("Encode failed: {}", e)))?;
        
        let raw = bitstream.to_vec();
        
        if raw.is_empty() {
            self.frame_count += 1;
            return Ok((Vec::new(), false));
        }
        
        // Fast keyframe detection
        let is_keyframe = self.is_keyframe(&raw);
        self.frame_count += 1;
        
        Ok((raw, is_keyframe))
    }

    /// Fast RGB to YUV420 conversion using SIMD-friendly patterns
    #[inline]
    fn rgb_to_yuv420_fast(&mut self, rgb: &[u8]) {
        let width = self.width as usize;
        let height = self.height as usize;
        let y_size = width * height;
        let uv_width = width / 2;
        
        // Split buffer into planes
        let (y_plane, uv_planes) = self.yuv_buffer.split_at_mut(y_size);
        let (u_plane, v_plane) = uv_planes.split_at_mut(y_size / 4);
        
        // Process 2x2 blocks for better cache locality
        for j in (0..height).step_by(2) {
            for i in (0..width).step_by(2) {
                // Process 4 pixels at once
                let mut sum_r = 0i32;
                let mut sum_g = 0i32;
                let mut sum_b = 0i32;
                
                for dy in 0..2 {
                    for dx in 0..2 {
                        let y_pos = j + dy;
                        let x_pos = i + dx;
                        if y_pos >= height || x_pos >= width { continue; }
                        
                        let rgb_idx = (y_pos * width + x_pos) * 3;
                        if rgb_idx + 2 >= rgb.len() { continue; }
                        
                        let r = rgb[rgb_idx] as i32;
                        let g = rgb[rgb_idx + 1] as i32;
                        let b = rgb[rgb_idx + 2] as i32;
                        
                        // Y plane - BT.601
                        let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
                        y_plane[y_pos * width + x_pos] = y.clamp(0, 255) as u8;
                        
                        sum_r += r;
                        sum_g += g;
                        sum_b += b;
                    }
                }
                
                // Average for UV (subsampled)
                let avg_r = sum_r >> 2;
                let avg_g = sum_g >> 2;
                let avg_b = sum_b >> 2;
                
                let u = ((-38 * avg_r - 74 * avg_g + 112 * avg_b + 128) >> 8) + 128;
                let v = ((112 * avg_r - 94 * avg_g - 18 * avg_b + 128) >> 8) + 128;
                
                let uv_idx = (j / 2) * uv_width + (i / 2);
                if uv_idx < u_plane.len() {
                    u_plane[uv_idx] = u.clamp(0, 255) as u8;
                    v_plane[uv_idx] = v.clamp(0, 255) as u8;
                }
            }
        }
    }

    /// Fast keyframe detection
    #[inline]
    fn is_keyframe(&self, data: &[u8]) -> bool {
        // Look for IDR NAL (type 5) or SPS (type 7)
        for i in 0..data.len().saturating_sub(5) {
            if data[i] == 0 && data[i+1] == 0 {
                let (offset, found) = if data[i+2] == 1 {
                    (i + 3, true)
                } else if data[i+2] == 0 && i + 3 < data.len() && data[i+3] == 1 {
                    (i + 4, true)
                } else {
                    (0, false)
                };
                
                if found && offset < data.len() {
                    let nal_type = data[offset] & 0x1F;
                    if nal_type == 5 || nal_type == 7 {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Zero-copy YUV buffer reference
struct YUVBufferRef<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
}

impl<'a> YUVSource for YUVBufferRef<'a> {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn strides(&self) -> (usize, usize, usize) {
        (self.width, self.width / 2, self.width / 2)
    }

    fn y(&self) -> &[u8] {
        &self.data[..self.width * self.height]
    }

    fn u(&self) -> &[u8] {
        let y_size = self.width * self.height;
        let u_size = y_size / 4;
        &self.data[y_size..y_size + u_size]
    }

    fn v(&self) -> &[u8] {
        let y_size = self.width * self.height;
        let u_size = y_size / 4;
        &self.data[y_size + u_size..]
    }
}
