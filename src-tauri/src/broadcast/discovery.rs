//! UDP Discovery Protocol
//! Allows teachers and students to find each other on the LAN

use std::collections::HashMap;
use std::net::{UdpSocket, SocketAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

pub const DISCOVERY_PORT: u16 = 5001;
pub const DISCOVERY_MAGIC: &[u8] = b"SCRSHARE";
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(2);
pub const PEER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub role: PeerRole,
    pub ip: String,
    pub stream_port: u16,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerRole {
    Teacher,
    Student,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum DiscoveryMessage {
    Announce(PeerInfo),
    Query,
    Response(PeerInfo),
}

pub struct DiscoveryService {
    socket: UdpSocket,
    local_info: PeerInfo,
    peers: Arc<Mutex<HashMap<String, (PeerInfo, Instant)>>>,
    running: Arc<Mutex<bool>>,
}

impl DiscoveryService {
    pub fn new(name: &str, role: PeerRole, stream_port: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", DISCOVERY_PORT))?;
        socket.set_broadcast(true)?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        
        // Get local IP
        let local_ip = get_local_ip().unwrap_or_else(|| "0.0.0.0".to_string());
        
        let local_info = PeerInfo {
            id: generate_id(),
            name: name.to_string(),
            role,
            ip: local_ip,
            stream_port,
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        
        log::info!("Discovery service created: {} ({:?}) at {}:{}", 
            local_info.name, local_info.role, local_info.ip, stream_port);
        
        Ok(Self {
            socket,
            local_info,
            peers: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(Mutex::new(false)),
        })
    }

    /// Start discovery service in background
    pub fn start(&self) -> std::io::Result<()> {
        *self.running.lock() = true;
        Ok(())
    }

    /// Stop discovery service
    pub fn stop(&self) {
        *self.running.lock() = false;
    }

    /// Send announcement broadcast
    pub fn announce(&self) -> std::io::Result<()> {
        let msg = DiscoveryMessage::Announce(self.local_info.clone());
        self.broadcast_message(&msg)
    }

    /// Send query to find peers
    pub fn query(&self) -> std::io::Result<()> {
        let msg = DiscoveryMessage::Query;
        self.broadcast_message(&msg)
    }

    /// Process incoming messages (call in a loop)
    pub fn process(&self) -> std::io::Result<Option<PeerInfo>> {
        let mut buf = [0u8; 2048];
        
        match self.socket.recv_from(&mut buf) {
            Ok((size, addr)) => {
                if size < DISCOVERY_MAGIC.len() {
                    return Ok(None);
                }
                
                // Check magic header
                if &buf[..DISCOVERY_MAGIC.len()] != DISCOVERY_MAGIC {
                    return Ok(None);
                }
                
                // Parse message
                let json_data = &buf[DISCOVERY_MAGIC.len()..size];
                if let Ok(msg) = serde_json::from_slice::<DiscoveryMessage>(json_data) {
                    return self.handle_message(msg, addr);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock 
                   || e.kind() == std::io::ErrorKind::TimedOut => {
                // No data available
            }
            Err(e) => {
                log::warn!("Discovery receive error: {}", e);
            }
        }
        
        Ok(None)
    }

    fn handle_message(&self, msg: DiscoveryMessage, addr: SocketAddr) -> std::io::Result<Option<PeerInfo>> {
        match msg {
            DiscoveryMessage::Announce(mut peer) => {
                // Update peer IP from actual source
                peer.ip = addr.ip().to_string();
                
                // Don't add ourselves
                if peer.id == self.local_info.id {
                    return Ok(None);
                }
                
                log::debug!("Discovered peer: {} ({:?}) at {}", peer.name, peer.role, peer.ip);
                
                let mut peers = self.peers.lock();
                let is_new = !peers.contains_key(&peer.id);
                peers.insert(peer.id.clone(), (peer.clone(), Instant::now()));
                
                if is_new {
                    return Ok(Some(peer));
                }
            }
            DiscoveryMessage::Query => {
                // Respond with our info
                let response = DiscoveryMessage::Response(self.local_info.clone());
                self.send_to(&response, addr)?;
            }
            DiscoveryMessage::Response(mut peer) => {
                peer.ip = addr.ip().to_string();
                
                if peer.id != self.local_info.id {
                    let mut peers = self.peers.lock();
                    let is_new = !peers.contains_key(&peer.id);
                    peers.insert(peer.id.clone(), (peer.clone(), Instant::now()));
                    
                    if is_new {
                        return Ok(Some(peer));
                    }
                }
            }
        }
        
        Ok(None)
    }

    fn broadcast_message(&self, msg: &DiscoveryMessage) -> std::io::Result<()> {
        let json = serde_json::to_vec(msg).unwrap();
        let mut packet = Vec::with_capacity(DISCOVERY_MAGIC.len() + json.len());
        packet.extend_from_slice(DISCOVERY_MAGIC);
        packet.extend_from_slice(&json);
        
        let broadcast_addr = format!("255.255.255.255:{}", DISCOVERY_PORT);
        self.socket.send_to(&packet, broadcast_addr)?;
        Ok(())
    }

    fn send_to(&self, msg: &DiscoveryMessage, addr: SocketAddr) -> std::io::Result<()> {
        let json = serde_json::to_vec(msg).unwrap();
        let mut packet = Vec::with_capacity(DISCOVERY_MAGIC.len() + json.len());
        packet.extend_from_slice(DISCOVERY_MAGIC);
        packet.extend_from_slice(&json);
        
        self.socket.send_to(&packet, addr)?;
        Ok(())
    }

    /// Get list of discovered peers
    pub fn get_peers(&self) -> Vec<PeerInfo> {
        let mut peers = self.peers.lock();
        let now = Instant::now();
        
        // Remove stale peers
        peers.retain(|_, (_, last_seen)| now.duration_since(*last_seen) < PEER_TIMEOUT);
        
        peers.values().map(|(p, _)| p.clone()).collect()
    }

    /// Get teachers only
    pub fn get_teachers(&self) -> Vec<PeerInfo> {
        self.get_peers()
            .into_iter()
            .filter(|p| p.role == PeerRole::Teacher)
            .collect()
    }

    /// Get students only
    pub fn get_students(&self) -> Vec<PeerInfo> {
        self.get_peers()
            .into_iter()
            .filter(|p| p.role == PeerRole::Student)
            .collect()
    }

    pub fn local_info(&self) -> &PeerInfo {
        &self.local_info
    }
}

fn get_local_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip().to_string())
}

fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", timestamp)
}
