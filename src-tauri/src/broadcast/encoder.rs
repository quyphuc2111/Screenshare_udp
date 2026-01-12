use openh264::encoder::{Encoder, EncoderConfig};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use std::time::Instant;

use super::capture::rgb_to_yuv420;
use super::types::BroadcastError;

pub struct H264Encoder {
    encoder: Encoder,
    width: u32,
    height: u32,
    fps: u32,
    bitrate_kbps: u32,
    frame_count: u64,
    keyframe_interval: u64,
    last_encode_time: Instant,
}

impl H264Encoder {
    pub fn new(width: u32, height: u32, fps: u32, bitrate_kbps: u32) -> Result<Self, BroadcastError> {
        let api = OpenH264API::from_source();
        
        // Optimize for low latency
        let config = EncoderConfig::new()
            .set_bitrate_bps(bitrate_kbps * 1000)
            .max_frame_rate(fps as f32)
            .enable_skip_frame(false);  // Don't skip frames
        
        let encoder = Encoder::with_api_config(api, config)
            .map_err(|e| BroadcastError::EncoderError(format!("Failed to create encoder: {}", e)))?;
        
        // More frequent keyframes for faster recovery
        let keyframe_interval = (fps * 2).max(30) as u64; // Keyframe every 2 seconds
        
        log::info!("H264 Encoder initialized: {}x{} @ {} fps, {} kbps, keyframe every {} frames", 
            width, height, fps, bitrate_kbps, keyframe_interval);
        
        Ok(Self {
            encoder,
            width,
            height,
            fps,
            bitrate_kbps,
            frame_count: 0,
            keyframe_interval,
            last_encode_time: Instant::now(),
        })
    }

    /// Encode RGB frame to H.264 NAL units
    pub fn encode(&mut self, rgb_data: &[u8]) -> Result<(Vec<u8>, bool), BroadcastError> {
        // Convert RGB to YUV420
        let yuv_data = rgb_to_yuv420(rgb_data, self.width as usize, self.height as usize);
        
        // Create YUV source
        let yuv_source = YUVBuffer {
            data: yuv_data,
            width: self.width as usize,
            height: self.height as usize,
        };
        
        // Force keyframe periodically by recreating encoder
        let force_keyframe = self.frame_count % self.keyframe_interval == 0;
        
        if force_keyframe {
            log::info!("Forcing keyframe at frame {} by recreating encoder", self.frame_count);
            
            // Recreate encoder to force keyframe
            let api = OpenH264API::from_source();
            let config = EncoderConfig::new()
                .set_bitrate_bps(self.bitrate_kbps * 1000)
                .max_frame_rate(self.fps as f32)
                .enable_skip_frame(false);
            
            self.encoder = Encoder::with_api_config(api, config)
                .map_err(|e| BroadcastError::EncoderError(format!("Failed to recreate encoder: {}", e)))?;
        }
        
        // Encode
        let bitstream = self.encoder.encode(&yuv_source)
            .map_err(|e| BroadcastError::EncoderError(format!("Encode failed: {}", e)))?;
        
        // Get raw bitstream
        let raw = bitstream.to_vec();
        
        if raw.is_empty() {
            self.frame_count += 1;
            return Ok((Vec::new(), false));
        }
        
        // Check for keyframe by looking at NAL types
        // IDR = 5, SPS = 7, PPS = 8
        let mut is_keyframe = false;
        let mut i = 0;
        while i < raw.len().saturating_sub(4) {
            // Look for start codes: 0x00 0x00 0x00 0x01 or 0x00 0x00 0x01
            let (start_code_len, found) = if raw[i] == 0 && raw[i+1] == 0 && raw[i+2] == 0 && raw[i+3] == 1 {
                (4, true)
            } else if raw[i] == 0 && raw[i+1] == 0 && raw[i+2] == 1 {
                (3, true)
            } else {
                (0, false)
            };
            
            if found && i + start_code_len < raw.len() {
                let nal_header = raw[i + start_code_len];
                let nal_type = nal_header & 0x1F;
                
                // Check for keyframe NAL types
                if nal_type == 5 {  // IDR
                    is_keyframe = true;
                    log::debug!("Found IDR NAL (type 5) at offset {}", i);
                } else if nal_type == 7 {  // SPS
                    is_keyframe = true;
                    log::debug!("Found SPS NAL (type 7) at offset {}", i);
                } else if nal_type == 8 {  // PPS
                    is_keyframe = true;
                    log::debug!("Found PPS NAL (type 8) at offset {}", i);
                }
                
                i += start_code_len;
            } else {
                i += 1;
            }
        }
        
        self.frame_count += 1;
        self.last_encode_time = Instant::now();
        
        if is_keyframe {
            log::info!("✓ Encoded KEYFRAME: frame {}, {} bytes", self.frame_count, raw.len());
        }
        
        Ok((raw, is_keyframe))
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn set_bitrate(&mut self, _bitrate_kbps: u32) {
        // OpenH264 doesn't support runtime bitrate change easily
        // Would need to recreate encoder
    }
}

/// YUV buffer wrapper for openh264
struct YUVBuffer {
    data: Vec<u8>,
    width: usize,
    height: usize,
}

impl YUVSource for YUVBuffer {
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
