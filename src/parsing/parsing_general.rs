use simd_json::BorrowedValue;
use simd_json::value::prelude::*;
use crate::parsing::{MessageParser, ParseError};
use crate::parsing::exchange_message_type::{DeribitMessage, MessageType};

impl MessageParser {
    pub fn parse_bytes(buffer: &mut [u8]) -> Result<DeribitMessage, ParseError> {
        let value = simd_json::to_borrowed_value(buffer)
            .map_err(|e| ParseError::JsonError(e.to_string()))?;
        Self::parse_owned(&value)
    }

    pub fn parse_owned(value: &BorrowedValue) -> Result<DeribitMessage, ParseError> {
        let msg_type = Self::detect_message_type_fast(&value)?;
        match msg_type {
            MessageType::Auth => Self::parse_auth_owned(&value),
            MessageType::Subscription => Self::parse_subscription_owned(&value),
            MessageType::TradeUpdate => Self::parse_trade_update_owned(&value),
            MessageType::OrderbookSnapshot => Self::parse_orderbook_snapshot_owned(&value),
            MessageType::OrderbookUpdate => Self::parse_orderbook_update_owned(&value),
            MessageType::Unknown => Ok(DeribitMessage::Unknown),
            MessageType::Pong => Self::parse_ping_pong(&value),
        }
    }

    fn detect_message_type_fast(value: &BorrowedValue) -> Result<MessageType, ParseError> {
        if let Some(method) = value.get("method").and_then(|m| m.as_str()) {
            if method == "subscription" {
                if let Some(channel) = value.get("params")
                    .and_then(|p| p.get("channel"))
                    .and_then(|c| c.as_str())
                {
                    if channel.starts_with("trades.") {
                        return Ok(MessageType::TradeUpdate);
                    }
                    if channel.starts_with("book.") {
                        if let Some(msg_type) = value.get("params")
                            .and_then(|p| p.get("data"))
                            .and_then(|d| d.get("type"))
                            .and_then(|t| t.as_str())
                        {
                            return match msg_type {
                                "snapshot" => Ok(MessageType::OrderbookSnapshot),
                                "change" => Ok(MessageType::OrderbookUpdate),
                                _ => Ok(MessageType::Unknown),
                            };
                        }
                    }
                }
            }
            return Ok(MessageType::Unknown);
        }

        if let Some(result) = value.get("result") {
            if result.get("access_token").is_some() {
                return Ok(MessageType::Auth);
            }
            if result.as_array().is_some() {
                return Ok(MessageType::Subscription);
            }
            if "pong" == result.to_string() {
                return Ok(MessageType::Pong);
            }
        }

        Ok(MessageType::Unknown)
    }
}