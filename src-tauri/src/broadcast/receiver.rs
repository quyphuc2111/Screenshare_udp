use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use crossbeam_channel::{bounded, Receiver, Sender};

use super::network::MulticastReceiver;
use super::types::{BroadcastError, FramePacket, PacketType, BroadcastConfig};

/// Reassembles fragmented frames
struct FrameAssembler {
    fragments: HashMap<u32, FrameFragments>,
    last_complete_frame: u32,
    timeout: Duration,
}

struct FrameFragments {
    data: Vec<Option<Vec<u8>>>,
    total: u16,
    received: u16,
    is_keyframe: bool,
    timestamp: u32,
    created_at: Instant,
}

impl FrameAssembler {
    fn new() -> Self {
        Self {
            fragments: HashMap::new(),
            last_complete_frame: 0,
            timeout: Duration::from_millis(500),
        }
    }

    fn add_packet(&mut self, packet: FramePacket) -> Option<(Vec<u8>, bool, u32)> {
        // Skip old frames
        if packet.frame_id < self.last_complete_frame.saturating_sub(10) {
            return None;
        }

        let is_keyframe = matches!(packet.packet_type, PacketType::KeyFrame);
        
        let entry = self.fragments.entry(packet.frame_id).or_insert_with(|| {
            FrameFragments {
                data: vec![None; packet.total_fragments as usize],
                total: packet.total_fragments,
                received: 0,
                is_keyframe,
                timestamp: packet.timestamp,
                created_at: Instant::now(),
            }
        });

        if !entry.is_keyframe && is_keyframe {
            entry.is_keyframe = true;
        }

        let idx = packet.fragment_idx as usize;
        if idx < entry.data.len() && entry.data[idx].is_none() {
            entry.data[idx] = Some(packet.data);
            entry.received += 1;
        }

        // Check if frame is complete
        if entry.received == entry.total {
            let frame_data: Vec<u8> = entry.data.iter()
                .filter_map(|d| d.as_ref())
                .flat_map(|d| d.iter().cloned())
                .collect();
            
            let is_key = entry.is_keyframe;
            let ts = entry.timestamp;
            
            self.last_complete_frame = packet.frame_id;
            self.fragments.remove(&packet.frame_id);
            
            // Cleanup old incomplete frames
            self.cleanup_old_frames();
            
            return Some((frame_data, is_key, ts));
        }

        None
    }

    fn cleanup_old_frames(&mut self) {
        let now = Instant::now();
        self.fragments.retain(|_, v| now.duration_since(v.created_at) < self.timeout);
    }
}

pub struct StreamReceiver {
    receiver: MulticastReceiver,
    decoder: Arc<Mutex<Decoder>>,
    assembler: FrameAssembler,
    #[allow(dead_code)]
    width: u32,
    #[allow(dead_code)]
    height: u32,
    #[allow(dead_code)]
    frame_tx: Sender<DecodedFrame>,
    #[allow(dead_code)]
    frame_rx: Receiver<DecodedFrame>,
    running: Arc<Mutex<bool>>,
    waiting_for_keyframe: bool,
}

#[derive(Clone)]
pub struct DecodedFrame {
    pub rgba_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub timestamp: u32,
    pub is_keyframe: bool,
}

impl StreamReceiver {
    pub fn new(config: &BroadcastConfig) -> Result<Self, BroadcastError> {
        let receiver = MulticastReceiver::new(&config.multicast_addr, config.port, None)?;
        
        let decoder = Decoder::new()
            .map_err(|e| BroadcastError::DecoderError(format!("Failed to create decoder: {}", e)))?;
        
        let (frame_tx, frame_rx) = bounded(3); // Small buffer to reduce latency
        
        Ok(Self {
            receiver,
            decoder: Arc::new(Mutex::new(decoder)),
            assembler: FrameAssembler::new(),
            width: config.width,
            height: config.height,
            frame_tx,
            frame_rx,
            running: Arc::new(Mutex::new(false)),
            waiting_for_keyframe: true,
        })
    }

    /// Process incoming packets and decode frames
    pub fn process(&mut self) -> Result<Option<DecodedFrame>, BroadcastError> {
        // Receive packets
        while let Some(packet) = self.receiver.receive_packet()? {
            if let Some((frame_data, is_keyframe, timestamp)) = self.assembler.add_packet(packet) {
                // Wait for keyframe before decoding
                if self.waiting_for_keyframe && !is_keyframe {
                    continue;
                }
                self.waiting_for_keyframe = false;
                
                // Decode H.264 frame
                if let Some(decoded) = self.decode_frame(&frame_data, is_keyframe, timestamp)? {
                    return Ok(Some(decoded));
                }
            }
        }
        
        Ok(None)
    }

    fn decode_frame(&self, h264_data: &[u8], is_keyframe: bool, timestamp: u32) -> Result<Option<DecodedFrame>, BroadcastError> {
        let mut decoder = self.decoder.lock();
        
        match decoder.decode(h264_data) {
            Ok(Some(yuv)) => {
                let (width, height) = yuv.dimensions();
                let mut rgba = vec![0u8; width * height * 4];
                
                // Convert YUV to RGBA
                yuv.write_rgba8(&mut rgba);
                
                Ok(Some(DecodedFrame {
                    rgba_data: rgba,
                    width: width as u32,
                    height: height as u32,
                    timestamp,
                    is_keyframe,
                }))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                log::warn!("Decode error: {}", e);
                Ok(None)
            }
        }
    }

    #[allow(dead_code)]
    pub fn frame_receiver(&self) -> Receiver<DecodedFrame> {
        self.frame_rx.clone()
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        *self.running.lock()
    }

    #[allow(dead_code)]
    pub fn stop(&self) {
        *self.running.lock() = false;
    }
}

/// Convert YUV420 to RGBA
#[allow(dead_code)]
pub fn yuv420_to_rgba(yuv: &[u8], width: usize, height: usize) -> Vec<u8> {
    let y_size = width * height;
    let uv_size = y_size / 4;
    
    let y_plane = &yuv[..y_size];
    let u_plane = &yuv[y_size..y_size + uv_size];
    let v_plane = &yuv[y_size + uv_size..];
    
    let mut rgba = vec![0u8; width * height * 4];
    
    for j in 0..height {
        for i in 0..width {
            let y_idx = j * width + i;
            let uv_idx = (j / 2) * (width / 2) + (i / 2);
            
            let y = y_plane[y_idx] as i32;
            let u = u_plane[uv_idx] as i32 - 128;
            let v = v_plane[uv_idx] as i32 - 128;
            
            // BT.601 conversion
            let r = y + ((351 * v) >> 8);
            let g = y - ((179 * v + 86 * u) >> 8);
            let b = y + ((443 * u) >> 8);
            
            let rgba_idx = y_idx * 4;
            rgba[rgba_idx] = r.clamp(0, 255) as u8;
            rgba[rgba_idx + 1] = g.clamp(0, 255) as u8;
            rgba[rgba_idx + 2] = b.clamp(0, 255) as u8;
            rgba[rgba_idx + 3] = 255;
        }
    }
    
    rgba
}
