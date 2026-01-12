//! Simple SFU (Selective Forwarding Unit) Server
//! Forwards RTP streams from Teacher to Students using WebRTC

use anyhow::Result;
use axum::{
    extract::{
        ws::{WebSocket, WebSocketUpgrade, Message},
        State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
};
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use uuid::Uuid;
use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors,
        media_engine::MediaEngine,
        APIBuilder,
    },
    ice_transport::{
        ice_candidate::RTCIceCandidateInit,
        ice_server::RTCIceServer,
    },
    peer_connection::{
        configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription,
        RTCPeerConnection,
    },
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::{track_local_static_rtp::TrackLocalStaticRTP, TrackLocal, TrackLocalWriter},
};

#[derive(Clone)]
struct AppState {
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
    video_track: Arc<TrackLocalStaticRTP>,
    rtp_sender: broadcast::Sender<Vec<u8>>,
}

struct PeerInfo {
    id: String,
    role: PeerRole,
    pc: Arc<RTCPeerConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum PeerRole {
    Teacher,
    Student,
}

#[derive(Debug, Serialize, Deserialize)]
struct SignalMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sdp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<RTCIceCandidateInit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<PeerRole>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    log::info!("🚀 Starting SFU Server...");
    
    // Create shared video track for forwarding
    let video_track = Arc::new(TrackLocalStaticRTP::new(
        RTCRtpCodecCapability {
            mime_type: "video/H264".to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: "".to_owned(),
            rtcp_feedback: vec![],
        },
        "video".to_owned(),
        "webrtc-rs".to_owned(),
    ));
    
    let (rtp_sender, _) = broadcast::channel::<Vec<u8>>(1000);
    
    let state = AppState {
        peers: Arc::new(RwLock::new(HashMap::new())),
        video_track,
        rtp_sender,
    };
    
    // Build router
    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/ws", get(websocket_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);
    
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    log::info!("📡 SFU listening on http://{}", addr);
    log::info!("WebSocket endpoint: ws://{}/ws", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    
    Ok(())
}

async fn root() -> &'static str {
    "SFU Server - WebRTC Selective Forwarding Unit"
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "sfu-server"
    }))
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let peer_id = Uuid::new_v4().to_string();
    log::info!("New peer connected: {}", peer_id);
    
    let (mut sender, mut receiver): (
        futures::stream::SplitSink<WebSocket, Message>,
        futures::stream::SplitStream<WebSocket>,
    ) = socket.split();
    
    // Create channel for sending messages from ICE callback
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();
    
    // Spawn task to forward messages from channel to websocket
    let sender_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });
    
    // Wait for role message
    let role = match receiver.next().await {
        Some(Ok(msg)) => {
            if let Message::Text(text) = msg {
                if let Ok(signal) = serde_json::from_str::<SignalMessage>(&text) {
                    signal.role
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    };
    
    let role = match role {
        Some(r) => r,
        None => {
            log::warn!("Peer {} didn't send role, disconnecting", peer_id);
            return;
        }
    };
    
    log::info!("Peer {} role: {:?}", peer_id, role);
    
    // Create PeerConnection
    let pc = match create_peer_connection(&state, &peer_id, &role, tx.clone()).await {
        Ok(pc) => pc,
        Err(e) => {
            log::error!("Failed to create peer connection: {}", e);
            return;
        }
    };
    
    // Store peer
    state.peers.write().insert(
        peer_id.clone(),
        PeerInfo {
            id: peer_id.clone(),
            role: role.clone(),
            pc: pc.clone(),
        },
    );
    
    // Handle signaling
    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            if let Ok(signal) = serde_json::from_str::<SignalMessage>(&text) {
                if let Err(e) = handle_signal(&pc, &signal, &tx).await {
                    log::error!("Signal error: {}", e);
                    break;
                }
            }
        }
    }
    
    // Cleanup
    sender_task.abort();
    state.peers.write().remove(&peer_id);
    let _ = pc.close().await;
    log::info!("Peer {} disconnected", peer_id);
}

async fn create_peer_connection(
    state: &AppState,
    peer_id: &str,
    role: &PeerRole,
    sender: tokio::sync::mpsc::UnboundedSender<Message>,
) -> Result<Arc<RTCPeerConnection>> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_default_codecs()?;
    
    let mut registry = webrtc::interceptor::registry::Registry::new();
    registry = register_default_interceptors(registry, &mut media_engine)?;
    
    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .build();
    
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };
    
    let pc = Arc::new(api.new_peer_connection(config).await?);
    
    // Add video track for students
    if matches!(role, PeerRole::Student) {
        pc.add_track(Arc::clone(&state.video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;
        log::info!("Added video track for student {}", peer_id);
    }
    
    // Handle incoming track from teacher
    if matches!(role, PeerRole::Teacher) {
        let video_track = Arc::clone(&state.video_track);
        let peer_id = peer_id.to_string();
        
        pc.on_track(Box::new(move |track, _, _| {
            let video_track = Arc::clone(&video_track);
            let peer_id = peer_id.clone();
            
            Box::pin(async move {
                log::info!("Teacher {} track received: {}", peer_id, track.codec().capability.mime_type);
                
                // Forward RTP packets
                while let Ok((rtp_packet, _)) = track.read_rtp().await {
                    // Write to shared track
                    if let Err(e) = video_track.write_rtp(&rtp_packet).await {
                        log::error!("Failed to write RTP: {}", e);
                        break;
                    }
                }
            })
        }));
    }
    
    // ICE candidate handler - send candidates to client
    pc.on_ice_candidate(Box::new(move |candidate| {
        let sender = sender.clone();
        Box::pin(async move {
            if let Some(candidate) = candidate {
                let msg = SignalMessage {
                    msg_type: "candidate".to_string(),
                    sdp: None,
                    candidate: Some(candidate.to_json().unwrap()),
                    role: None,
                };
                if let Ok(json) = serde_json::to_string(&msg) {
                    let _ = sender.send(Message::Text(json));
                }
            }
        })
    }));
    
    // Connection state handler
    let peer_id_clone = peer_id.to_string();
    pc.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
        log::info!("Peer {} connection state: {}", peer_id_clone, state);
        Box::pin(async {})
    }));
    
    Ok(pc)
}

async fn handle_signal(
    pc: &Arc<RTCPeerConnection>,
    signal: &SignalMessage,
    sender: &tokio::sync::mpsc::UnboundedSender<Message>,
) -> Result<()> {
    match signal.msg_type.as_str() {
        "offer" => {
            if let Some(sdp) = &signal.sdp {
                let offer = RTCSessionDescription::offer(sdp.clone())?;
                pc.set_remote_description(offer).await?;
                
                let answer = pc.create_answer(None).await?;
                pc.set_local_description(answer.clone()).await?;
                
                let response = SignalMessage {
                    msg_type: "answer".to_string(),
                    sdp: Some(answer.sdp),
                    candidate: None,
                    role: None,
                };
                
                sender.send(Message::Text(serde_json::to_string(&response)?))?;
            }
        }
        "answer" => {
            if let Some(sdp) = &signal.sdp {
                let answer = RTCSessionDescription::answer(sdp.clone())?;
                pc.set_remote_description(answer).await?;
            }
        }
        "candidate" => {
            if let Some(candidate) = &signal.candidate {
                log::info!("Adding ICE candidate from client");
                pc.add_ice_candidate(candidate.clone()).await?;
            }
        }
        _ => {}
    }
    
    Ok(())
}
