use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;


#[derive(Debug, Error, Clone)]
pub enum DeribitError {
    #[error("RPC error {code}: {message}")]
    Rpc { code: i32, message: String },
    #[error("Invalid message format: {0}")]
    InvalidFormat(String),
    #[error("Missing field: {0}")]
    MissingField(String),
    #[error("Invalid message type: {0}")]
    InvalidMessageType(String),
    #[error("WebSocket connection failed: {0}")]
    ConnectionError(String),
    #[error("Authentication failed: {0}")]
    AuthError(String),
    #[error("Subscription error: {0}")]
    SubscriptionError(String),
    #[error("WebSocket error: {0}")]
    WsError(String),
    #[error("Timeout waiting for response")]
    Timeout,
    #[error("Response channel closed")]
    ChannelClosed,
}

#[derive(Debug, Clone)]
pub struct SubscriptionResult {
    pub channels: Vec<String>,
    pub success: bool,
}

#[derive(Debug, Clone)]
pub enum DeribitMessage {
    AuthResponse(AuthResult),
    SubscriptionConfirmation(SubscriptionResult),
    OrderBookUpdate(OrderBookData),
    Heartbeat,
    Error(String),
    ChannelData {
        channel: String,
        data: Value,
    },
    Trades {
        channel: String,
        trades: Vec<Trade>,
    },
}

#[derive(Debug, Clone)]
pub enum DeribitResponse {
    Notification(DeribitMessage),
    RpcReply {
        id: String,
        result: Value,
    },
    RpcError {
        id: String,
        error: DeribitError,
        code: i32,
    },
    InternalError {
        id: String,
        error: DeribitError,
    },
}

#[derive(Debug, Clone)]
pub struct AuthResult {
    pub access_token: String,
    pub expires_in: u64,
}

#[derive(Debug, Clone)]
pub struct OrderBookData {
    pub instrument: String,
    pub bids: Vec<(f32, f32)>,
    pub asks: Vec<(f32, f32)>,
    pub timestamp: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Trade {
    pub timestamp: u64,
    pub price: f32,
    pub amount: f32,
    pub direction: String,
}


