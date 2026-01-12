//! WebRTC Teacher - Captures screen and publishes to SFU

use anyhow::Result;
use std::sync::Arc;
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
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::{track_local_static_rtp::TrackLocalStaticRTP, TrackLocal, TrackLocalWriter},
    rtp::packet::Packet as RtpPacket,
};

use crate::broadcast::{ScreenCapture, H264Encoder};
use super::signaling::{SignalingClient, PeerRole, SignalMessage};

pub struct WebRTCTeacher {
    pc: Arc<RTCPeerConnection>,
    video_track: Arc<TrackLocalStaticRTP>,
}

impl WebRTCTeacher {
    pub async fn new(sfu_url: &str) -> Result<Self> {
        // Create signaling client
        let mut signaling = SignalingClient::connect(sfu_url, PeerRole::Teacher).await?;
        
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
        
        // Create peer connection
        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        };
        
        let pc = Arc::new(api.new_peer_connection(config).await?);
        
        // Create video track
        let video_track = Arc::new(TrackLocalStaticRTP::new(
            RTCRtpCodecCapability {
                mime_type: "video/H264".to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
                rtcp_feedback: vec![],
            },
            "video".to_owned(),
            "teacher-stream".to_owned(),
        ));
        
        // Add track to peer connection
        pc.add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;
        
        // Handle connection state
        pc.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
            log::info!("Teacher connection state: {}", state);
            Box::pin(async {})
        }));
        
        // Create offer
        let offer = pc.create_offer(None).await?;
        pc.set_local_description(offer.clone()).await?;
        
        // Send offer to SFU
        signaling
            .send(&SignalMessage {
                msg_type: "offer".to_string(),
                sdp: Some(offer.sdp),
                candidate: None,
                role: None,
            })
            .await?;
        
        // Wait for answer
        if let Some(answer_msg) = signaling.receive().await? {
            if answer_msg.msg_type == "answer" {
                if let Some(sdp) = answer_msg.sdp {
                    let answer = webrtc::peer_connection::sdp::session_description::RTCSessionDescription::answer(sdp)?;
                    pc.set_remote_description(answer).await?;
                }
            }
        }
        
        // Handle ICE candidates
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
        
        Ok(Self { pc, video_track })
    }
    
    pub async fn start_capture(&self, fps: u32, bitrate_kbps: u32) -> Result<()> {
        let video_track = Arc::clone(&self.video_track);
        
        // Spawn blocking thread for capture (scrap is not Send)
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            
            let mut capture = match ScreenCapture::new(fps) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to create capture: {}", e);
                    return;
                }
            };
            
            let (width, height) = capture.dimensions();
            let mut encoder = match H264Encoder::new(width, height, fps, bitrate_kbps) {
                Ok(e) => e,
                Err(e) => {
                    log::error!("Failed to create encoder: {}", e);
                    return;
                }
            };
            
            let mut frame_count = 0u64;
            
            loop {
                // Capture frame
                match capture.capture_frame() {
                    Ok(Some(rgb_data)) => {
                        // Encode
                        match encoder.encode(&rgb_data) {
                            Ok((h264_data, _is_keyframe)) => {
                                if !h264_data.is_empty() {
                                    // Create RTP packet from H.264 data
                                    let rtp_packet = RtpPacket {
                                        header: webrtc::rtp::header::Header {
                                            version: 2,
                                            padding: false,
                                            extension: false,
                                            marker: true,
                                            payload_type: 96,
                                            sequence_number: frame_count as u16,
                                            timestamp: (frame_count * 3000) as u32,
                                            ssrc: 12345,
                                            ..Default::default()
                                        },
                                        payload: h264_data.into(),
                                    };
                                    
                                    // Send via WebRTC (async)
                                    let video_track = Arc::clone(&video_track);
                                    rt.block_on(async move {
                                        if let Err(e) = video_track.write_rtp(&rtp_packet).await {
                                            log::error!("Failed to write RTP: {}", e);
                                        }
                                    });
                                    
                                    frame_count += 1;
                                    if frame_count % 30 == 0 {
                                        log::info!("Sent {} frames via WebRTC", frame_count);
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Encode error: {}", e);
                            }
                        }
                    }
                    Ok(None) => {
                        // No frame ready
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(e) => {
                        log::error!("Capture error: {}", e);
                        break;
                    }
                }
                
                // Frame rate control
                std::thread::sleep(std::time::Duration::from_millis(1000 / fps as u64));
            }
        });
        
        Ok(())
    }
    
    pub async fn close(&self) -> Result<()> {
        self.pc.close().await?;
        Ok(())
    }
}
