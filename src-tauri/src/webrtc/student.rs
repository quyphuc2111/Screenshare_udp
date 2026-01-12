//! WebRTC Student - Receives stream from SFU and decodes

use anyhow::Result;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors,
        media_engine::MediaEngine,
        APIBuilder,
    },
    ice_transport::ice_server::RTCIceServer,
    peer_connection::{
        configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        RTCPeerConnection,
    },
};

use crate::broadcast::H264Decoder;
use super::signaling::{SignalingClient, PeerRole, SignalMessage};

#[derive(Clone, serde::Serialize)]
struct FrameData {
    width: u32,
    height: u32,
    data: String,
}

pub struct WebRTCStudent {
    pc: Arc<RTCPeerConnection>,
}

impl WebRTCStudent {
    pub async fn new(sfu_url: &str, app: AppHandle) -> Result<Self> {
        // Create signaling client
        let mut signaling = SignalingClient::connect(sfu_url, PeerRole::Student).await?;
        
        // Create media engine
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs()?;
        
        // Create interceptor registry
        let mut registry = webrtc::interceptor::registry::Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)?;
        
        // Create API
        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();
        
        // Create peer connection - LAN optimized (no STUN)
        let config = RTCConfiguration {
            ice_servers: vec![], // No STUN for LAN
            ..Default::default()
        };
        
        let pc = Arc::new(api.new_peer_connection(config).await?);
        
        // Handle connection state
        pc.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
            log::info!("Student connection state: {}", state);
            Box::pin(async move {})
        }));
        
        // Handle incoming track
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let app = app.clone();
            
            Box::pin(async move {
                log::info!("Received track: {}", track.codec().capability.mime_type);
                
                let mut decoder = H264Decoder::new().unwrap();
                let mut frame_count = 0u64;
                
                // Read RTP packets
                while let Ok((rtp_packet, _)) = track.read_rtp().await {
                    let payload = rtp_packet.payload.clone();
                    
                    // Decode H.264
                    match decoder.decode(&payload) {
                        Ok(Some(frame)) => {
                            frame_count += 1;
                            
                            // Send to frontend
                            let frame_data = FrameData {
                                width: frame.width,
                                height: frame.height,
                                data: BASE64.encode(&frame.rgba_data),
                            };
                            
                            if let Err(e) = app.emit("video-frame", &frame_data) {
                                log::error!("Failed to emit frame: {}", e);
                            }
                            
                            if frame_count % 30 == 0 {
                                log::info!("Decoded {} frames", frame_count);
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            log::warn!("Decode error: {}", e);
                        }
                    }
                }
            })
        }));
        
        // Wait for offer from SFU
        if let Some(offer_msg) = signaling.receive().await? {
            if offer_msg.msg_type == "offer" {
                if let Some(sdp) = offer_msg.sdp {
                    let offer = webrtc::peer_connection::sdp::session_description::RTCSessionDescription::offer(sdp)?;
                    pc.set_remote_description(offer).await?;
                    
                    // Create answer
                    let answer = pc.create_answer(None).await?;
                    pc.set_local_description(answer.clone()).await?;
                    
                    // Send answer
                    signaling
                        .send(&SignalMessage {
                            msg_type: "answer".to_string(),
                            sdp: Some(answer.sdp),
                            candidate: None,
                            role: None,
                        })
                        .await?;
                }
            }
        }
        
        // Handle ICE candidates from student
        let signaling_clone = Arc::new(tokio::sync::Mutex::new(signaling));
        let signaling_for_ice = Arc::clone(&signaling_clone);
        
        pc.on_ice_candidate(Box::new(move |candidate| {
            let signaling = Arc::clone(&signaling_for_ice);
            Box::pin(async move {
                if let Some(candidate) = candidate {
                    let mut sig = signaling.lock().await;
                    let _ = sig
                        .send(&SignalMessage {
                            msg_type: "candidate".to_string(),
                            sdp: None,
                            candidate: Some(candidate.to_json().unwrap()),
                            role: None,
                        })
                        .await;
                }
            })
        }));
        
        // Spawn task to receive ICE candidates from SFU
        let pc_clone = Arc::clone(&pc);
        let signaling_for_recv = Arc::clone(&signaling_clone);
        tokio::spawn(async move {
            loop {
                let mut sig = signaling_for_recv.lock().await;
                match sig.receive().await {
                    Ok(Some(msg)) => {
                        if msg.msg_type == "candidate" {
                            if let Some(candidate) = msg.candidate {
                                log::info!("Student received ICE candidate from SFU");
                                let _ = pc_clone.add_ice_candidate(candidate).await;
                            }
                        }
                    }
                    _ => break,
                }
            }
        });
        
        Ok(Self { pc })
    }
    
    pub async fn close(&self) -> Result<()> {
        self.pc.close().await?;
        Ok(())
    }
}
