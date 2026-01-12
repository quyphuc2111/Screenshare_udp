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
    frame_count: u64,
    keyframe_interval: u64,
    last_encode_time: Instant,
}

impl H264Encoder {
    pub fn new(width: u32, height: u32, fps: u32, bitrate_kbps: u32) -> Result<Self, BroadcastError> {
        let api = OpenH264API::from_source();
        
        let config = EncoderConfig::new()
            .set_bitrate_bps(bitrate_kbps * 1000)
            .max_frame_rate(fps as f32)
            .enable_skip_frame(true);
        
        let encoder = Encoder::with_api_config(api, config)
            .map_err(|e| BroadcastError::EncoderError(format!("Failed to create encoder: {}", e)))?;
        
        Ok(Self {
            encoder,
            width,
            height,
            frame_count: 0,
            keyframe_interval: (fps * 2) as u64, // Keyframe every 2 seconds
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
        
        // Force keyframe periodically
        let force_keyframe = self.frame_count % self.keyframe_interval == 0;
        
        // Encode
        let bitstream = if force_keyframe {
            self.encoder.encode_at(&yuv_source, openh264::Timestamp::ZERO)
        } else {
            self.encoder.encode(&yuv_source)
        };
        
        let bitstream = bitstream
            .map_err(|e| BroadcastError::EncoderError(format!("Encode failed: {}", e)))?;
        
        // Collect NAL units
        let mut encoded_data = Vec::new();
        let mut is_keyframe = false;
        
        // Get raw bitstream
        let raw = bitstream.to_vec();
        if !raw.is_empty() {
            // Check for keyframe by looking at NAL types
            // IDR = 5, SPS = 7, PPS = 8
            for i in 0..raw.len().saturating_sub(4) {
                if raw[i] == 0 && raw[i+1] == 0 && raw[i+2] == 0 && raw[i+3] == 1 {
                    if i + 4 < raw.len() {
                        let nal_type = raw[i+4] & 0x1F;
                        if nal_type == 5 || nal_type == 7 || nal_type == 8 {
                            is_keyframe = true;
                        }
                    }
                }
            }
            encoded_data = raw;
        }
        
        self.frame_count += 1;
        self.last_encode_time = Instant::now();
        
        Ok((encoded_data, is_keyframe))
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
