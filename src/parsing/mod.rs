pub mod parsing_general;
pub mod parsing_admin;
pub mod parsing_trade;
pub mod parsing_orderbook;
pub mod exchange_message_type;
pub mod parsing_fast;
pub mod parsing_fast_orderbook;

use simd_json::borrowed::Value as BorrowedValue;
use simd_json::derived::ValueObjectAccess;
use simd_json::base::ValueAsScalar;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum ParseError {
    #[error("JSON parsing failed: {0}")]
    JsonError(String),
    #[error("Missing required field: {0}")]
    MissingField(String),
    #[error("Invalid message format: {0}")]
    InvalidFormat(String),
    #[error("Unknown message type")]
    UnknownMessageType,
    #[error("Could not match field: {0}")]
    FastParserTrade(String),
    #[error("The buffer is too short, size: {0}")]
    BufferTooShort(String),
    #[error("Encounter this issue: {0}")]
    FastParserOrderBook(String),
}


pub struct MessageParser;

impl MessageParser {
    #[inline]
    fn get_string(value: &BorrowedValue, field: &str) -> Result<String, ParseError> {
        value.get(field)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ParseError::MissingField(field.to_string()))
    }

    #[inline]
    fn get_u64(value: &BorrowedValue, field: &str) -> Result<u64, ParseError> {
        value.get(field)
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ParseError::MissingField(field.to_string()))
    }

    #[inline]
    fn get_f64(value: &BorrowedValue, field: &str) -> Result<f64, ParseError> {
        value.get(field)
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ParseError::MissingField(field.to_string()))
    }

    #[inline]
    fn get_usize(value: &BorrowedValue, field: &str) -> Result<usize, ParseError> {
        value.get(field)
            .and_then(|v| v.as_usize())
            .ok_or_else(|| ParseError::MissingField(field.to_string()))
    }

}


