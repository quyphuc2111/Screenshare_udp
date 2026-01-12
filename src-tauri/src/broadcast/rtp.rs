//! RTP (Real-time Transport Protocol) implementation for H.264
//! RFC 3550 (RTP) + RFC 6184 (H.264 payload)

use std::time::{SystemTime, UNIX_EPOCH};

pub const RTP_VERSION: u8 = 2;
pub const RTP_PAYLOAD_TYPE_H264: u8 = 96; // Dynamic payload type for H.264
pub const RTP_HEADER_SIZE: usize = 12;
pub const MAX_RTP_PAYLOAD: usize = 1400; // MTU safe

/// RTP Header (12 bytes)
/// ```text
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |V=2|P|X|  CC   |M|     PT      |       sequence number         |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |                           timestamp                           |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |           synchronization source (SSRC) identifier            |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
#[derive(Debug, Clone)]
pub struct RtpHeader {
    pub version: u8,        // 2 bits, always 2
    pub padding: bool,      // 1 bit
    pub extension: bool,    // 1 bit
    pub csrc_count: u8,     // 4 bits
    pub marker: bool,       // 1 bit - end of frame for H.264
    pub payload_type: u8,   // 7 bits
    pub sequence: u16,      // 16 bits
    pub timestamp: u32,     // 32 bits
    pub ssrc: u32,          // 32 bits - synchronization source
}

impl RtpHeader {
    pub fn new(ssrc: u32) -> Self {
        Self {
            version: RTP_VERSION,
            padding: false,
            extension: false,
            csrc_count: 0,
            marker: false,
            payload_type: RTP_PAYLOAD_TYPE_H264,
            sequence: 0,
            timestamp: 0,
            ssrc,
        }
    }

    pub fn serialize(&self) -> [u8; RTP_HEADER_SIZE] {
        let mut buf = [0u8; RTP_HEADER_SIZE];
        
        // Byte 0: V(2) P(1) X(1) CC(4)
        buf[0] = (self.version << 6) 
               | ((self.padding as u8) << 5)
               | ((self.extension as u8) << 4)
               | (self.csrc_count & 0x0F);
        
        // Byte 1: M(1) PT(7)
        buf[1] = ((self.marker as u8) << 7) | (self.payload_type & 0x7F);
        
        // Bytes 2-3: Sequence number
        buf[2] = (self.sequence >> 8) as u8;
        buf[3] = self.sequence as u8;
        
        // Bytes 4-7: Timestamp
        buf[4] = (self.timestamp >> 24) as u8;
        buf[5] = (self.timestamp >> 16) as u8;
        buf[6] = (self.timestamp >> 8) as u8;
        buf[7] = self.timestamp as u8;
        
        // Bytes 8-11: SSRC
        buf[8] = (self.ssrc >> 24) as u8;
        buf[9] = (self.ssrc >> 16) as u8;
        buf[10] = (self.ssrc >> 8) as u8;
        buf[11] = self.ssrc as u8;
        
        buf
    }

    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < RTP_HEADER_SIZE {
            return None;
        }
        
        let version = (data[0] >> 6) & 0x03;
        if version != RTP_VERSION {
            return None;
        }
        
        Some(Self {
            version,
            padding: (data[0] >> 5) & 0x01 == 1,
            extension: (data[0] >> 4) & 0x01 == 1,
            csrc_count: data[0] & 0x0F,
            marker: (data[1] >> 7) & 0x01 == 1,
            payload_type: data[1] & 0x7F,
            sequence: ((data[2] as u16) << 8) | (data[3] as u16),
            timestamp: ((data[4] as u32) << 24) 
                     | ((data[5] as u32) << 16) 
                     | ((data[6] as u32) << 8) 
                     | (data[7] as u32),
            ssrc: ((data[8] as u32) << 24) 
                | ((data[9] as u32) << 16) 
                | ((data[10] as u32) << 8) 
                | (data[11] as u32),
        })
    }
}

/// H.264 NAL Unit types
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NalType {
    Slice,      // Non-IDR slice (1)
    SliceA,     // (2)
    SliceB,     // (3)
    SliceC,     // (4)
    Idr,        // IDR slice - keyframe (5)
    Sei,        // Supplemental enhancement info (6)
    Sps,        // Sequence parameter set (7)
    Pps,        // Picture parameter set (8)
    Aud,        // Access unit delimiter (9)
    FuA,        // Fragmentation unit A (28)
    FuB,        // Fragmentation unit B (29)
    Unknown(u8),
}

impl From<u8> for NalType {
    fn from(val: u8) -> Self {
        match val & 0x1F {
            1 => NalType::Slice,
            2 => NalType::SliceA,
            3 => NalType::SliceB,
            4 => NalType::SliceC,
            5 => NalType::Idr,
            6 => NalType::Sei,
            7 => NalType::Sps,
            8 => NalType::Pps,
            9 => NalType::Aud,
            28 => NalType::FuA,
            29 => NalType::FuB,
            n => NalType::Unknown(n),
        }
    }
}

/// RTP Packetizer for H.264
pub struct RtpPacketizer {
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
    clock_rate: u32,
}

impl RtpPacketizer {
    pub fn new() -> Self {
        // Generate random SSRC
        let ssrc = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u32;
        
        Self {
            ssrc,
            sequence: 0,
            timestamp: 0,
            clock_rate: 90000, // Standard for video
        }
    }

    /// Packetize H.264 NAL units into RTP packets
    /// Returns list of RTP packets ready to send
    pub fn packetize(&mut self, h264_data: &[u8], frame_time_ms: u32) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        
        // Update timestamp (90kHz clock)
        self.timestamp = (frame_time_ms as u64 * self.clock_rate as u64 / 1000) as u32;
        
        // Find NAL units (separated by 0x00 0x00 0x00 0x01 or 0x00 0x00 0x01)
        let nal_units = self.find_nal_units(h264_data);
        
        for (i, nal) in nal_units.iter().enumerate() {
            let is_last = i == nal_units.len() - 1;
            let nal_packets = self.packetize_nal(nal, is_last);
            packets.extend(nal_packets);
        }
        
        packets
    }

    fn find_nal_units<'a>(&self, data: &'a [u8]) -> Vec<&'a [u8]> {
        let mut units = Vec::new();
        let mut start = 0;
        let mut i = 0;
        
        while i < data.len() {
            // Look for start code (0x00 0x00 0x01 or 0x00 0x00 0x00 0x01)
            if i + 3 < data.len() && data[i] == 0 && data[i+1] == 0 {
                let (code_len, found) = if data[i+2] == 1 {
                    (3, true)
                } else if i + 4 < data.len() && data[i+2] == 0 && data[i+3] == 1 {
                    (4, true)
                } else {
                    (0, false)
                };
                
                if found {
                    if start < i && i > start {
                        // Save previous NAL
                        if start + 3 < i {
                            let prev_start = if data[start] == 0 && data[start+1] == 0 && data[start+2] == 1 {
                                start + 3
                            } else if start + 4 <= data.len() && data[start] == 0 && data[start+1] == 0 && data[start+2] == 0 && data[start+3] == 1 {
                                start + 4
                            } else {
                                start
                            };
                            if prev_start < i {
                                units.push(&data[prev_start..i]);
                            }
                        }
                    }
                    start = i;
                    i += code_len;
                    continue;
                }
            }
            i += 1;
        }
        
        // Last NAL unit
        if start < data.len() {
            let nal_start = if data[start] == 0 && data[start+1] == 0 && data[start+2] == 1 {
                start + 3
            } else if start + 4 <= data.len() && data[start] == 0 && data[start+1] == 0 && data[start+2] == 0 && data[start+3] == 1 {
                start + 4
            } else {
                start
            };
            if nal_start < data.len() {
                units.push(&data[nal_start..]);
            }
        }
        
        units
    }

    fn packetize_nal(&mut self, nal: &[u8], is_last_nal: bool) -> Vec<Vec<u8>> {
        let mut packets = Vec::new();
        
        if nal.is_empty() {
            return packets;
        }
        
        if nal.len() <= MAX_RTP_PAYLOAD {
            // Single NAL unit packet
            let mut header = RtpHeader::new(self.ssrc);
            header.sequence = self.sequence;
            header.timestamp = self.timestamp;
            header.marker = is_last_nal; // Marker bit set on last packet of frame
            
            let mut packet = Vec::with_capacity(RTP_HEADER_SIZE + nal.len());
            packet.extend_from_slice(&header.serialize());
            packet.extend_from_slice(nal);
            
            packets.push(packet);
            self.sequence = self.sequence.wrapping_add(1);
        } else {
            // Fragmentation Unit A (FU-A)
            let nal_header = nal[0];
            let nal_type = nal_header & 0x1F;
            let nri = nal_header & 0x60;
            
            // FU indicator: same NRI, type = 28 (FU-A)
            let fu_indicator = nri | 28;
            
            let payload = &nal[1..]; // Skip NAL header
            let chunks: Vec<&[u8]> = payload.chunks(MAX_RTP_PAYLOAD - 2).collect();
            
            for (i, chunk) in chunks.iter().enumerate() {
                let is_first = i == 0;
                let is_last = i == chunks.len() - 1;
                
                // FU header: S(1) E(1) R(1) Type(5)
                let fu_header = ((is_first as u8) << 7) 
                              | ((is_last as u8) << 6) 
                              | nal_type;
                
                let mut header = RtpHeader::new(self.ssrc);
                header.sequence = self.sequence;
                header.timestamp = self.timestamp;
                header.marker = is_last && is_last_nal;
                
                let mut packet = Vec::with_capacity(RTP_HEADER_SIZE + 2 + chunk.len());
                packet.extend_from_slice(&header.serialize());
                packet.push(fu_indicator);
                packet.push(fu_header);
                packet.extend_from_slice(chunk);
                
                packets.push(packet);
                self.sequence = self.sequence.wrapping_add(1);
            }
        }
        
        packets
    }
}

/// RTP Depacketizer for H.264
pub struct RtpDepacketizer {
    expected_sequence: Option<u16>,
    current_frame: Vec<u8>,
    current_timestamp: u32,
    fu_buffer: Vec<u8>,
    fu_started: bool,
}

impl RtpDepacketizer {
    pub fn new() -> Self {
        Self {
            expected_sequence: None,
            current_frame: Vec::new(),
            current_timestamp: 0,
            fu_buffer: Vec::new(),
            fu_started: false,
        }
    }

    /// Process RTP packet and return complete H.264 frame if available
    pub fn depacketize(&mut self, rtp_data: &[u8]) -> Option<Vec<u8>> {
        let header = RtpHeader::parse(rtp_data)?;
        
        if header.payload_type != RTP_PAYLOAD_TYPE_H264 {
            return None;
        }
        
        let payload = &rtp_data[RTP_HEADER_SIZE..];
        if payload.is_empty() {
            return None;
        }
        
        // Check sequence
        if let Some(expected) = self.expected_sequence {
            if header.sequence != expected {
                log::warn!("RTP sequence gap: expected {}, got {}", expected, header.sequence);
                // Reset on sequence gap
                self.fu_buffer.clear();
                self.fu_started = false;
            }
        }
        self.expected_sequence = Some(header.sequence.wrapping_add(1));
        
        // New frame?
        if header.timestamp != self.current_timestamp {
            self.current_frame.clear();
            self.current_timestamp = header.timestamp;
        }
        
        // Parse NAL unit type
        let nal_type = NalType::from(payload[0]);
        
        match nal_type {
            NalType::FuA => {
                // Fragmentation Unit A
                if payload.len() < 2 {
                    return None;
                }
                
                let fu_indicator = payload[0];
                let fu_header = payload[1];
                let is_start = (fu_header >> 7) & 1 == 1;
                let is_end = (fu_header >> 6) & 1 == 1;
                let nal_type = fu_header & 0x1F;
                
                if is_start {
                    self.fu_buffer.clear();
                    // Reconstruct NAL header
                    let nal_header = (fu_indicator & 0xE0) | nal_type;
                    self.fu_buffer.push(nal_header);
                    self.fu_started = true;
                }
                
                if self.fu_started {
                    self.fu_buffer.extend_from_slice(&payload[2..]);
                }
                
                if is_end && self.fu_started {
                    // Add start code and NAL
                    self.current_frame.extend_from_slice(&[0, 0, 0, 1]);
                    self.current_frame.extend_from_slice(&self.fu_buffer);
                    self.fu_buffer.clear();
                    self.fu_started = false;
                }
            }
            _ => {
                // Single NAL unit
                self.current_frame.extend_from_slice(&[0, 0, 0, 1]);
                self.current_frame.extend_from_slice(payload);
            }
        }
        
        // Return frame if marker bit is set (end of frame)
        if header.marker && !self.current_frame.is_empty() {
            let frame = std::mem::take(&mut self.current_frame);
            return Some(frame);
        }
        
        None
    }
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
