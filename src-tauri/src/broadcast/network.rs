//! Network layer for RTP streaming over UDP

use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;

use super::rtp::{RtpPacketizer, RtpDepacketizer};
use super::types::{BroadcastError, NetworkMode};

pub const STREAM_PORT: u16 = 5000;
pub const MULTICAST_ADDR: &str = "239.255.0.1";
pub const RTP_HEADER_SIZE: usize = 12;

/// RTP Sender - sends H.264 frames as RTP packets
pub struct RtpSender {
    socket: UdpSocket,
    target: SocketAddr,
    packetizer: RtpPacketizer,
    frame_count: u64,
}

impl RtpSender {
    pub fn new(port: u16, mode: NetworkMode) -> Result<Self, BroadcastError> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        
        socket.set_reuse_address(true)?;
        socket.set_broadcast(true)?;
        
        if mode == NetworkMode::Multicast {
            socket.set_multicast_ttl_v4(1)?;
            socket.set_multicast_loop_v4(true)?;
        }
        
        // Bind to any port
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        socket.bind(&bind_addr.into())?;
        
        // Set send buffer
        socket.set_send_buffer_size(2 * 1024 * 1024)?;
        
        let target: SocketAddr = match mode {
            NetworkMode::Broadcast => format!("255.255.255.255:{}", port).parse().unwrap(),
            NetworkMode::Multicast => format!("{}:{}", MULTICAST_ADDR, port).parse().unwrap(),
        };
        
        log::info!("RTP Sender ready: {:?} mode, target: {}", mode, target);
        
        Ok(Self {
            socket: socket.into(),
            target,
            packetizer: RtpPacketizer::new(),
            frame_count: 0,
        })
    }

    /// Send H.264 frame as RTP packets
    pub fn send_frame(&mut self, h264_data: &[u8], timestamp_ms: u32) -> Result<usize, BroadcastError> {
        let packets = self.packetizer.packetize(h264_data, timestamp_ms);
        let mut total_bytes = 0;
        
        if packets.is_empty() {
            log::warn!("No RTP packets generated from {} bytes H264 data", h264_data.len());
            return Ok(0);
        }
        
        for packet in &packets {
            match self.socket.send_to(packet, self.target) {
                Ok(n) => total_bytes += n,
                Err(e) => {
                    log::error!("Send error: {}", e);
                    return Err(BroadcastError::NetworkError(e.to_string()));
                }
            }
        }
        
        self.frame_count += 1;
        
        // Log every 30 frames
        if self.frame_count % 30 == 0 {
            log::info!("Sent frame {}: {} packets, {} bytes to {}", 
                self.frame_count, packets.len(), total_bytes, self.target);
        }
        
        Ok(total_bytes)
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

/// RTP Receiver - receives RTP packets and reassembles H.264 frames
pub struct RtpReceiver {
    socket: Arc<Mutex<UdpSocket>>,
    depacketizer: RtpDepacketizer,
    buffer: Vec<u8>,
}

impl RtpReceiver {
    pub fn new(port: u16, mode: NetworkMode) -> Result<Self, BroadcastError> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        
        socket.set_reuse_address(true)?;
        socket.set_broadcast(true)?;
        
        #[cfg(not(windows))]
        socket.set_reuse_port(true)?;
        
        // Bind to port
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        socket.bind(&bind_addr.into())?;
        
        log::info!("RTP Receiver bound to 0.0.0.0:{}", port);
        
        // Join multicast if needed
        if mode == NetworkMode::Multicast {
            let multicast_ip: Ipv4Addr = MULTICAST_ADDR.parse().unwrap();
            socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::UNSPECIFIED)
                .map_err(|e| BroadcastError::NetworkError(format!("Join multicast failed: {}", e)))?;
            log::info!("Joined multicast group: {}", MULTICAST_ADDR);
        }
        
        // Set receive buffer
        socket.set_recv_buffer_size(4 * 1024 * 1024)?;
        
        // Blocking with timeout
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        
        log::info!("RTP Receiver ready: {:?} mode, port: {}", mode, port);
        
        Ok(Self {
            socket: Arc::new(Mutex::new(socket.into())),
            depacketizer: RtpDepacketizer::new(),
            buffer: vec![0u8; 2048],
        })
    }

    /// Receive and process RTP packets, returns complete H.264 frame if available
    pub fn receive_frame(&mut self) -> Result<Option<Vec<u8>>, BroadcastError> {
        let socket = self.socket.lock();
        
        // Try to receive packets
        match socket.recv_from(&mut self.buffer) {
            Ok((size, addr)) => {
                // Log first few packets
                static PACKET_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let count = PACKET_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                
                if count < 10 || count % 100 == 0 {
                    log::info!("RTP packet #{}: {} bytes from {}", count, size, addr);
                }
                
                if size < RTP_HEADER_SIZE {
                    log::warn!("Packet too small: {} bytes", size);
                    return Ok(None);
                }
                
                // Process RTP packet
                if let Some(frame) = self.depacketizer.depacketize(&self.buffer[..size]) {
                    log::info!("Frame assembled: {} bytes", frame.len());
                    return Ok(Some(frame));
                }
                
                Ok(None)
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock 
                   || e.kind() == std::io::ErrorKind::TimedOut => {
                Ok(None)
            }
            Err(e) => {
                log::error!("Socket error: {}", e);
                Err(BroadcastError::NetworkError(e.to_string()))
            }
        }
    }
}

impl Clone for RtpReceiver {
    fn clone(&self) -> Self {
        Self {
            socket: self.socket.clone(),
            depacketizer: RtpDepacketizer::new(),
            buffer: vec![0u8; 2048],
        }
    }
}
