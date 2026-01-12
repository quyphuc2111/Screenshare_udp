//! WebSocket signaling client

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream,
};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerRole {
    Teacher,
    Student,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SignalMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<RTCIceCandidateInit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<PeerRole>,
}

pub struct SignalingClient {
    ws: WsStream,
}

impl SignalingClient {
    pub async fn connect(url: &str, role: PeerRole) -> Result<Self> {
        let (ws_stream, _) = connect_async(url).await?;
        let mut client = Self { ws: ws_stream };
        
        // Send role
        let role_msg = SignalMessage {
            msg_type: "role".to_string(),
            sdp: None,
            candidate: None,
            role: Some(role),
        };
        client.send(&role_msg).await?;
        
        Ok(client)
    }
    
    pub async fn send(&mut self, msg: &SignalMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;
        self.ws.send(Message::Text(json)).await?;
        Ok(())
    }
    
    pub async fn receive(&mut self) -> Result<Option<SignalMessage>> {
        if let Some(msg) = self.ws.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                let signal: SignalMessage = serde_json::from_str(&text)?;
                return Ok(Some(signal));
            }
        }
        Ok(None)
    }
}
