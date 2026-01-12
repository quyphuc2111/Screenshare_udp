//! RTP handling using rtp-rs library
//! H.264 packetization according to RFC 6184

use rtp_rs::RtpReader;
use std::time::{SystemTime, UNIX_EPOCH};

pub const RTP_PAYLOAD_TYPE_H264: u8 = 96;
pub const MAX_RTP_PAYLOAD: usize = 1400;
pub const RTP_CLOCK_RATE: u32 = 90000;

/// RTP Packetizer for H.264 using rtp-rs
pub struct RtpPacketizer {
    ssrc: u32,
    sequence: u16,
    clock_rate: u32,
}

impl RtpPacketizer {
    pub fn new() -> Self {
        let ssrc = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u32;
        
        Self {
            ssrc,
            sequence: 0,
            clock_rate: RTP_CLOCK_RATE,
        }
    }

    /// Packetize H.264 frame into RTP packets
    pub fn packetize(&mut self, h264_data: &[u8], timestamp_ms: u32) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        let timestamp = (timestamp_ms as u64 * self.clock_rate as u64 / 1000) as u32;
        
        // Find NAL units
        let nal_units = find_nal_units(h264_data);
        
        for (i, nal) in nal_units.iter().enumerate() {
            let is_last_nal = i == nal_units.len() - 1;
            
            if nal.len() <= MAX_RTP_PAYLOAD {
                // Single NAL unit mode
                let packet = self.build_packet(nal, timestamp, is_last_nal);
                packets.push(packet);
            } else {
                // FU-A fragmentation
                let fu_packets = self.fragment_nal(nal, timestamp, is_last_nal);
                packets.extend(fu_packets);
            }
        }
        
        packets
    }

    fn build_packet(&mut self, payload: &[u8], timestamp: u32, marker: bool) -> Vec<u8> {
        let seq = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        
        let mut packet = Vec::with_capacity(12 + payload.len());
        
        // RTP Header (12 bytes)
        // V=2, P=0, X=0, CC=0
        packet.push(0x80);
        // M bit + PT
        packet.push(if marker { 0x80 | RTP_PAYLOAD_TYPE_H264 } else { RTP_PAYLOAD_TYPE_H264 });
        // Sequence number
        packet.push((seq >> 8) as u8);
        packet.push(seq as u8);
        // Timestamp
        packet.push((timestamp >> 24) as u8);
        packet.push((timestamp >> 16) as u8);
        packet.push((timestamp >> 8) as u8);
        packet.push(timestamp as u8);
        // SSRC
        packet.push((self.ssrc >> 24) as u8);
        packet.push((self.ssrc >> 16) as u8);
        packet.push((self.ssrc >> 8) as u8);
        packet.push(self.ssrc as u8);
        
        // Payload
        packet.extend_from_slice(payload);
        
        packet
    }

    fn fragment_nal(&mut self, nal: &[u8], timestamp: u32, is_last_nal: bool) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        
        if nal.is_empty() {
            return packets;
        }
        
        let nal_header = nal[0];
        let nal_type = nal_header & 0x1F;
        let nri = nal_header & 0x60;
        
        // FU indicator: NRI + type 28 (FU-A)
        let fu_indicator = nri | 28;
        
        let payload = &nal[1..]; // Skip original NAL header
        let max_fragment = MAX_RTP_PAYLOAD - 2; // Reserve 2 bytes for FU indicator + header
        
        let chunks: Vec<&[u8]> = payload.chunks(max_fragment).collect();
        
        for (i, chunk) in chunks.iter().enumerate() {
            let is_first = i == 0;
            let is_last = i == chunks.len() - 1;
            
            // FU header: S E R Type
            let fu_header = ((is_first as u8) << 7) 
                          | ((is_last as u8) << 6) 
                          | nal_type;
            
            let mut fu_payload = Vec::with_capacity(2 + chunk.len());
            fu_payload.push(fu_indicator);
            fu_payload.push(fu_header);
            fu_payload.extend_from_slice(chunk);
            
            let marker = is_last && is_last_nal;
            let packet = self.build_packet(&fu_payload, timestamp, marker);
            packets.push(packet);
        }
        
        packets
    }
}

/// RTP Depacketizer for H.264
pub struct RtpDepacketizer {
    current_frame: Vec<u8>,
    current_timestamp: Option<u32>,
    fu_buffer: Vec<u8>,
    fu_started: bool,
    last_seq: Option<u16>,
}

impl RtpDepacketizer {
    pub fn new() -> Self {
        Self {
            current_frame: Vec::new(),
            current_timestamp: None,
            fu_buffer: Vec::new(),
            fu_started: false,
            last_seq: None,
        }
    }

    /// Process RTP packet, returns complete H.264 frame when marker bit is set
    pub fn depacketize(&mut self, rtp_data: &[u8]) -> Option<Vec<u8>> {
        if rtp_data.len() < 12 {
            return None;
        }
        
        // Parse RTP header manually for reliability
        let version = (rtp_data[0] >> 6) & 0x03;
        if version != 2 {
            log::warn!("Invalid RTP version: {}", version);
            return None;
        }
        
        let marker = (rtp_data[1] >> 7) & 0x01 == 1;
        let payload_type = rtp_data[1] & 0x7F;
        let sequence = ((rtp_data[2] as u16) << 8) | (rtp_data[3] as u16);
        let timestamp = ((rtp_data[4] as u32) << 24) 
                      | ((rtp_data[5] as u32) << 16) 
                      | ((rtp_data[6] as u32) << 8) 
                      | (rtp_data[7] as u32);
        
        if payload_type != RTP_PAYLOAD_TYPE_H264 {
            return None;
        }
        
        // Check sequence
        if let Some(last) = self.last_seq {
            let expected = last.wrapping_add(1);
            if sequence != expected {
                log::warn!("RTP sequence gap: expected {}, got {}", expected, sequence);
                // Reset FU state on gap
                self.fu_buffer.clear();
                self.fu_started = false;
            }
        }
        self.last_seq = Some(sequence);
        
        let payload = &rtp_data[12..];
        if payload.is_empty() {
            return None;
        }
        
        // New timestamp = new frame
        if self.current_timestamp != Some(timestamp) {
            if !self.current_frame.is_empty() && self.current_timestamp.is_some() {
                // Previous frame wasn't completed, discard
                log::debug!("Discarding incomplete frame");
            }
            self.current_frame.clear();
            self.current_timestamp = Some(timestamp);
        }
        
        // Parse NAL unit type
        let nal_type = payload[0] & 0x1F;
        
        match nal_type {
            28 => {
                // FU-A
                if payload.len() < 2 {
                    return None;
                }
                
                let fu_indicator = payload[0];
                let fu_header = payload[1];
                let start = (fu_header >> 7) & 1 == 1;
                let end = (fu_header >> 6) & 1 == 1;
                let original_nal_type = fu_header & 0x1F;
                
                if start {
                    self.fu_buffer.clear();
                    // Reconstruct NAL header
                    let nal_header = (fu_indicator & 0xE0) | original_nal_type;
                    self.fu_buffer.push(nal_header);
                    self.fu_started = true;
                }
                
                if self.fu_started && payload.len() > 2 {
                    self.fu_buffer.extend_from_slice(&payload[2..]);
                }
                
                if end && self.fu_started {
                    // Complete NAL unit
                    self.current_frame.extend_from_slice(&[0, 0, 0, 1]);
                    self.current_frame.extend_from_slice(&self.fu_buffer);
                    self.fu_buffer.clear();
                    self.fu_started = false;
                }
            }
            1..=23 => {
                // Single NAL unit
                self.current_frame.extend_from_slice(&[0, 0, 0, 1]);
                self.current_frame.extend_from_slice(payload);
            }
            24 => {
                // STAP-A (aggregation)
                let mut offset = 1;
                while offset + 2 < payload.len() {
                    let size = ((payload[offset] as usize) << 8) | (payload[offset + 1] as usize);
                    offset += 2;
                    if offset + size <= payload.len() {
                        self.current_frame.extend_from_slice(&[0, 0, 0, 1]);
                        self.current_frame.extend_from_slice(&payload[offset..offset + size]);
                        offset += size;
                    } else {
                        break;
                    }
                }
            }
            _ => {
                log::debug!("Unknown NAL type: {}", nal_type);
            }
        }
        
        // Return frame if marker bit is set
        if marker && !self.current_frame.is_empty() {
            let frame = std::mem::take(&mut self.current_frame);
            self.current_timestamp = None;
            log::debug!("Complete frame: {} bytes", frame.len());
            return Some(frame);
        }
        
        None
    }
}

/// Find NAL units in H.264 bitstream
fn find_nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut units = Vec::new();
    let mut i = 0;
    let mut start = None;
    
    while i < data.len() {
        // Look for start code
        if i + 2 < data.len() && data[i] == 0 && data[i + 1] == 0 {
            let (code_len, found) = if data[i + 2] == 1 {
                (3, true)
            } else if i + 3 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                (4, true)
            } else {
                (0, false)
            };
            
            if found {
                if let Some(s) = start {
                    // Save previous NAL (without trailing zeros)
                    let end = i;
                    if end > s {
                        units.push(&data[s..end]);
                    }
                }
                start = Some(i + code_len);
                i += code_len;
                continue;
            }
        }
        i += 1;
    }
    
    // Last NAL
    if let Some(s) = start {
        if s < data.len() {
            units.push(&data[s..]);
        }
    }
    
    units
}

impl Default for RtpPacketizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for RtpDepacketizer {
    fn default() -> Self {
        Self::new()
    }
}
