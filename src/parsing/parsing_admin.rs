use serde::Deserialize;
use simd_json::BorrowedValue;
use simd_json::value::prelude::*;
use crate::parsing::{MessageParser, ParseError};
use crate::parsing::exchange_message_type::DeribitMessage;

#[derive(Debug, Clone, Deserialize)]
pub struct AuthMessage {
    pub jsonrpc: String,
    pub id: u64,
    pub result: AuthResult,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthResult {
    pub access_token: String,
    pub expires_in: u64,
    pub token_type: String,
    pub scope: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubscriptionMessage {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PongMessage {
    pub us_in: usize,
    pub us_out: usize,
    pub us_diff: usize,
}

impl MessageParser {
    pub fn parse_auth_owned(value: &BorrowedValue) -> Result<DeribitMessage, ParseError> {
        let jsonrpc = Self::get_string(value, "jsonrpc")?;
        let id = Self::get_u64(value, "id")?;
        let result = value.get("result")
            .ok_or_else(|| ParseError::MissingField("result".to_string()))?;

        let access_token = Self::get_string(result, "access_token")?;
        let expires_in = Self::get_u64(result, "expires_in")?;
        let token_type = Self::get_string(result, "token_type")?;
        let scope = Self::get_string(result, "scope")?;

        Ok(DeribitMessage::Auth(AuthMessage {
            jsonrpc,
            id,
            result: AuthResult {
                access_token,
                expires_in,
                token_type,
                scope,
            },
        }))
    }

    pub fn parse_subscription_owned(value: &BorrowedValue) -> Result<DeribitMessage, ParseError> {
        let jsonrpc = Self::get_string(value, "jsonrpc")?;
        let id = Self::get_u64(value, "id")?;
        let result = value.get("result")
            .ok_or_else(|| ParseError::MissingField("result".to_string()))?
            .as_array()
            .ok_or_else(|| ParseError::InvalidFormat("result not array".to_string()))?;

        let channels: Result<Vec<String>, ParseError> = result
            .iter()
            .map(|v| v.as_str()
                .ok_or_else(|| ParseError::InvalidFormat("channel not string".to_string()))
                .map(|s| s.to_string()))
            .collect();

        Ok(DeribitMessage::Subscription(SubscriptionMessage {
            jsonrpc,
            id,
            result: channels?,
        }))
    }

    pub fn parse_ping_pong(value: &BorrowedValue) -> Result<DeribitMessage, ParseError> {
        let us_in = Self::get_usize(value, "usIn")?;
        let us_out = Self::get_usize(value, "usOut")?;
        let us_diff = Self::get_usize(value, "usDiff")?;
        Ok(DeribitMessage::Pong(PongMessage {
            us_in,
            us_out,
            us_diff,
        }))
    }
}