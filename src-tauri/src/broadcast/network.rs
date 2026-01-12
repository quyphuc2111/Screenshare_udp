use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use parking_lot::Mutex;

use super::types::{BroadcastError, FramePacket, PacketType, MAX_PACKET_SIZE, FRAME_HEADER_SIZE};

pub struct MulticastSender {
    socket: UdpSocket,
    multicast_addr: SocketAddrV4,
    frame_id: u32,
}

impl MulticastSender {
    pub fn new(multicast_addr: &str, port: u16) -> Result<Self, BroadcastError> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        
        // Set socket options for multicast
        socket.set_reuse_address(true)?;
        socket.set_multicast_ttl_v4(1)?; // LAN only
        socket.set_nonblocking(false)?;
        
        // Bind to any address
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
        socket.bind(&bind_addr.into())?;
        
        let multicast_ip: Ipv4Addr = multicast_addr.parse()
            .map_err(|_| BroadcastError::ConfigError("Invalid multicast address".into()))?;
        
        let multicast_addr = SocketAddrV4::new(multicast_ip, port);
        
        Ok(Self {
            socket: socket.into(),
            multicast_addr,
            frame_id: 0,
        })
    }

    /// Send encoded frame data, fragmenting if necessary
    pub fn send_frame(&mut self, data: &[u8], is_keyframe: bool) -> Result<(), BroadcastError> {
        let max_payload = MAX_PACKET_SIZE - FRAME_HEADER_SIZE;
        let total_fragments = ((data.len() + max_payload - 1) / max_payload) as u16;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u32;
        
        let base_type = if is_keyframe {
            PacketType::KeyFrame
        } else {
            PacketType::DeltaFrame
        };
        
        for (idx, chunk) in data.chunks(max_payload).enumerate() {
            let packet_type = if total_fragments == 1 {
                base_type
            } else if idx == total_fragments as usize - 1 {
                PacketType::FrameEnd
            } else if idx == 0 {
                base_type
            } else {
                PacketType::FrameFragment
            };
            
            let packet = FramePacket {
                frame_id: self.frame_id,
                fragment_idx: idx as u16,
                total_fragments,
                packet_type,
                timestamp,
                data: chunk.to_vec(),
            };
            
            let serialized = packet.serialize();
            self.socket.send_to(&serialized, self.multicast_addr)?;
        }
        
        self.frame_id = self.frame_id.wrapping_add(1);
        Ok(())
    }

    pub fn frame_id(&self) -> u32 {
        self.frame_id
    }
}

pub struct MulticastReceiver {
    socket: Arc<Mutex<UdpSocket>>,
    buffer: Vec<u8>,
}

impl MulticastReceiver {
    pub fn new(multicast_addr: &str, port: u16, interface: Option<&str>) -> Result<Self, BroadcastError> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        
        socket.set_reuse_address(true)?;
        #[cfg(not(windows))]
        socket.set_reuse_port(true)?;
        
        // Bind to multicast port
        let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        socket.bind(&bind_addr.into())?;
        
        // Join multicast group
        let multicast_ip: Ipv4Addr = multicast_addr.parse()
            .map_err(|_| BroadcastError::ConfigError("Invalid multicast address".into()))?;
        
        let interface_ip: Ipv4Addr = interface
            .map(|s| s.parse().unwrap_or(Ipv4Addr::UNSPECIFIED))
            .unwrap_or(Ipv4Addr::UNSPECIFIED);
        
        socket.join_multicast_v4(&multicast_ip, &interface_ip)?;
        
        // Set receive buffer size (important for high throughput)
        socket.set_recv_buffer_size(4 * 1024 * 1024)?; // 4MB
        
        // Non-blocking for async receive
        socket.set_nonblocking(true)?;
        
        Ok(Self {
            socket: Arc::new(Mutex::new(socket.into())),
            buffer: vec![0u8; MAX_PACKET_SIZE + FRAME_HEADER_SIZE],
        })
    }

    /// Receive a packet (non-blocking)
    pub fn receive_packet(&mut self) -> Result<Option<FramePacket>, BroadcastError> {
        let socket = self.socket.lock();
        
        match socket.recv_from(&mut self.buffer) {
            Ok((size, _addr)) => {
                if let Some(packet) = FramePacket::deserialize(&self.buffer[..size]) {
                    Ok(Some(packet))
                } else {
                    Ok(None)
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Ok(None)
            }
            Err(e) => Err(BroadcastError::NetworkError(e.to_string())),
        }
    }

    pub fn socket(&self) -> Arc<Mutex<UdpSocket>> {
        self.socket.clone()
    }
}

impl Clone for MulticastReceiver {
    fn clone(&self) -> Self {
        Self {
            socket: self.socket.clone(),
            buffer: vec![0u8; MAX_PACKET_SIZE + FRAME_HEADER_SIZE],
        }
    }
}
