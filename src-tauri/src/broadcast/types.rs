use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MULTICAST_ADDR: &str = "239.255.0.1";
pub const MULTICAST_PORT: u16 = 5000;
pub const MAX_PACKET_SIZE: usize = 1400; // MTU safe size
pub const FRAME_HEADER_SIZE: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastConfig {
    pub multicast_addr: String,
    pub port: u16,
    pub fps: u32,
    pub quality: u32, // 0-51 for H.264 QP
    pub width: u32,
    pub height: u32,
}

impl Default for BroadcastConfig {
    fn default() -> Self {
        Self {
            multicast_addr: MULTICAST_ADDR.to_string(),
            port: MULTICAST_PORT,
            fps: 15,
            quality: 28, // Good balance
            width: 1920,
            height: 1080,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastStats {
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub frame_count: u64,
    pub dropped_frames: u64,
    pub cpu_usage: f32,
    pub latency_ms: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    KeyFrame = 0x01,
    DeltaFrame = 0x02,
    FrameFragment = 0x03,
    FrameEnd = 0x04,
}

/// RTP-like packet header for frame transmission
/// [0-3]   Frame ID (u32)
/// [4-5]   Fragment index (u16)
/// [6-7]   Total fragments (u16)
/// [8]     Packet type
/// [9-11]  Reserved
/// [12-15] Timestamp (u32)
#[derive(Debug, Clone)]
pub struct FramePacket {
    pub frame_id: u32,
    pub fragment_idx: u16,
    pub total_fragments: u16,
    pub packet_type: PacketType,
    pub timestamp: u32,
    pub data: Vec<u8>,
}

impl FramePacket {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(FRAME_HEADER_SIZE + self.data.len());
        buf.extend_from_slice(&self.frame_id.to_be_bytes());
        buf.extend_from_slice(&self.fragment_idx.to_be_bytes());
        buf.extend_from_slice(&self.total_fragments.to_be_bytes());
        buf.push(self.packet_type as u8);
        buf.extend_from_slice(&[0u8; 3]); // Reserved
        buf.extend_from_slice(&self.timestamp.to_be_bytes());
        buf.extend_from_slice(&self.data);
        buf
    }

    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.len() < FRAME_HEADER_SIZE {
            return None;
        }
        
        let frame_id = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let fragment_idx = u16::from_be_bytes([data[4], data[5]]);
        let total_fragments = u16::from_be_bytes([data[6], data[7]]);
        let packet_type = match data[8] {
            0x01 => PacketType::KeyFrame,
            0x02 => PacketType::DeltaFrame,
            0x03 => PacketType::FrameFragment,
            0x04 => PacketType::FrameEnd,
            _ => return None,
        };
        let timestamp = u32::from_be_bytes([data[12], data[13], data[14], data[15]]);
        
        Some(Self {
            frame_id,
            fragment_idx,
            total_fragments,
            packet_type,
            timestamp,
            data: data[FRAME_HEADER_SIZE..].to_vec(),
        })
    }
}

#[derive(Error, Debug)]
pub enum BroadcastError {
    #[error("Screen capture error: {0}")]
    CaptureError(String),
    
    #[error("Encoder error: {0}")]
    EncoderError(String),
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("Decoder error: {0}")]
    DecoderError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

impl From<std::io::Error> for BroadcastError {
    fn from(e: std::io::Error) -> Self {
        BroadcastError::NetworkError(e.to_string())
    }
}
