//! H.264 Decoder wrapper

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

use super::types::BroadcastError;

pub struct H264Decoder {
    decoder: Decoder,
    frame_count: u64,
}

impl H264Decoder {
    pub fn new() -> Result<Self, BroadcastError> {
        let decoder = Decoder::new()
            .map_err(|e| BroadcastError::DecoderError(format!("Failed to create decoder: {}", e)))?;
        
        Ok(Self {
            decoder,
            frame_count: 0,
        })
    }

    /// Decode H.264 data to RGBA
    pub fn decode(&mut self, h264_data: &[u8]) -> Result<Option<DecodedFrame>, BroadcastError> {
        match self.decoder.decode(h264_data) {
            Ok(Some(yuv)) => {
                let (width, height) = yuv.dimensions();
                let mut rgba = vec![0u8; width * height * 4];
                
                // Convert YUV to RGBA
                yuv.write_rgba8(&mut rgba);
                
                self.frame_count += 1;
                
                Ok(Some(DecodedFrame {
                    rgba_data: rgba,
                    width: width as u32,
                    height: height as u32,
                }))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                log::warn!("Decode error: {}", e);
                Err(BroadcastError::DecoderError(e.to_string()))
            }
        }
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

#[derive(Clone)]
pub struct DecodedFrame {
    pub rgba_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
