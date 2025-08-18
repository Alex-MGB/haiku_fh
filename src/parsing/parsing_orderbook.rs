use serde::Deserialize;
use simd_json::BorrowedValue;
use simd_json::value::prelude::*;
use crate::parsing::{MessageParser, ParseError};
use crate::parsing::exchange_message_type::DeribitMessage;


// THIS IS DEFAULT PARSER, NOT USED ANYMORE

#[derive(Debug, Clone, Deserialize)]
pub struct OrderbookSnapshotMessage {
    pub params: OrderbookSnapshotData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrderbookSnapshotData {
    pub timestamp: u64,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub change_id: u64,
    pub instrument_name: String,
    pub bids: Vec<(String, f32, f32)>,
    pub asks: Vec<(String, f32, f32)>,
}


#[derive(Debug, Clone, Deserialize)]
pub struct OrderbookUpdateMessage {
    pub params: OrderbookUpdateData,
}


#[derive(Debug, Clone, Deserialize)]
pub struct OrderbookUpdateData {
    pub timestamp: u64,
    #[serde(rename = "type")]
    pub msg_type: String, // "change"
    pub change_id: u64,
    pub instrument_name: String,
    pub bids: Vec<(String, f32, f32)>,
    pub asks: Vec<(String, f32, f32)>,
    pub prev_change_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderbookAction {
    New,
    Change,
    Delete,
}

impl OrderbookAction {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "new" => Some(Self::New),
            "change" => Some(Self::Change),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        match bytes {
            b"new" => Some(Self::New),
            b"change" => Some(Self::Change),
            b"delete" => Some(Self::Delete),
            _ => None,
        }
    }

    #[inline]
    pub fn from_number(number: i32) -> Option<Self> {
        match number {
            0 => Some(Self::New),
            1 => Some(Self::Change),
            2 => Some(Self::Delete),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OrderbookLevel {
    pub action: OrderbookAction,
    pub price: f32,
    pub size: f32,
}

impl OrderbookLevel {
    pub fn from_vec(action: String, price: f32, size: f32) -> Option<Self> {
        let action = OrderbookAction::from_str(&action)?;
        Some(Self { action, price, size })
    }

    pub fn from_action_number(action: i32, price: f32, size: f32) -> Option<Self> {
        let action = OrderbookAction::from_number(action)?;
        Some(Self { action, price, size })
    }
}

impl MessageParser {
    pub fn parse_orderbook_snapshot_owned(value: &BorrowedValue) -> Result<DeribitMessage, ParseError> {
        let params = value.get("params")
            .ok_or_else(|| ParseError::MissingField("params".to_string()))?;
        let data = params.get("data")
            .ok_or_else(|| ParseError::MissingField("data".to_string()))?;

        let channel = Self::get_string(params, "channel")?;
        let timestamp = Self::get_u64(data, "timestamp")?;
        let msg_type = Self::get_string(data, "type")?;
        let change_id = Self::get_u64(data, "change_id")?;
        let instrument_name = Self::get_string(data, "instrument_name")?;

        let bids = Self::parse_orderbook_levels_owned(data, "bids")?;
        let asks = Self::parse_orderbook_levels_owned(data, "asks")?;

        Ok(DeribitMessage::OrderbookSnapshot(OrderbookSnapshotMessage {
            params: OrderbookSnapshotData {
                    timestamp,
                    msg_type,
                    change_id,
                    instrument_name,
                    bids,
                    asks,
                },
        }))
    }

    pub fn parse_orderbook_update_owned(value: &BorrowedValue) -> Result<DeribitMessage, ParseError> {
        let params = value.get("params")
            .ok_or_else(|| ParseError::MissingField("params".to_string()))?;
        let data = params.get("data")
            .ok_or_else(|| ParseError::MissingField("data".to_string()))?;
        let timestamp = Self::get_u64(data, "timestamp")?;
        let msg_type = Self::get_string(data, "type")?;
        let change_id = Self::get_u64(data, "change_id")?;
        let instrument_name = Self::get_string(data, "instrument_name")?;
        let prev_change_id = data.get("prev_change_id").and_then(|v| v.as_u64());

        let bids = Self::parse_orderbook_levels_owned(data, "bids")?;
        let asks = Self::parse_orderbook_levels_owned(data, "asks")?;

        Ok(DeribitMessage::OrderbookUpdate(OrderbookUpdateMessage {
            params: OrderbookUpdateData {
                    timestamp,
                    msg_type,
                    change_id,
                    instrument_name,
                    bids,
                    asks,
                    prev_change_id,
                },
        }))
    }

    #[inline]
    fn parse_orderbook_levels_owned(data: &BorrowedValue, field: &str) -> Result<Vec<(String, f32, f32)>, ParseError> {
        let levels = data.get(field)
            .ok_or_else(|| ParseError::MissingField(field.to_string()))?
            .as_array()
            .ok_or_else(|| ParseError::InvalidFormat(format!("{} not array", field)))?;

        levels.iter().map(|level| {
            let arr = level.as_array()
                .ok_or_else(|| ParseError::InvalidFormat("level not array".to_string()))?;

            if arr.len() != 3 {
                return Err(ParseError::InvalidFormat("level array wrong size".to_string()));
            }

            Ok((
                arr[0].as_str().ok_or_else(|| ParseError::InvalidFormat("action not string".to_string()))?.to_string(),
                arr[1].as_f64().ok_or_else(|| ParseError::InvalidFormat("price not number".to_string()))? as f32,
                arr[2].as_f64().ok_or_else(|| ParseError::InvalidFormat("size not number".to_string()))? as f32,
            ))
        }).collect()
    }
}