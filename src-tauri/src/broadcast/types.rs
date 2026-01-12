use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkMode {
    Multicast,
    Broadcast,
}

impl Default for NetworkMode {
    fn default() -> Self {
        NetworkMode::Broadcast
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamConfig {
    pub port: u16,
    pub fps: u32,
    pub quality: u32,
    pub network_mode: NetworkMode,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            port: 5000,
            fps: 15,
            quality: 28,
            network_mode: NetworkMode::Broadcast,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamStats {
    pub fps: f32,
    pub bitrate_kbps: f32,
    pub frame_count: u64,
    pub packets_sent: u64,
    pub packets_lost: u64,
    pub latency_ms: f32,
}

impl Default for StreamStats {
    fn default() -> Self {
        Self {
            fps: 0.0,
            bitrate_kbps: 0.0,
            frame_count: 0,
            packets_sent: 0,
            packets_lost: 0,
            latency_ms: 0.0,
        }
    }
}

#[derive(Error, Debug)]
pub enum BroadcastError {
    #[error("Screen capture error: {0}")]
    CaptureError(String),
    
    #[error("Encoder error: {0}")]
    EncoderError(String),
    
    #[error("Decoder error: {0}")]
    DecoderError(String),
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

impl From<std::io::Error> for BroadcastError {
    fn from(e: std::io::Error) -> Self {
        BroadcastError::NetworkError(e.to_string())
    }
}
