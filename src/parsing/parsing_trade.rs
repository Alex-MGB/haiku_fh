use serde::Deserialize;
use simd_json::BorrowedValue;
use simd_json::value::prelude::*;
use crate::parsing::{MessageParser, ParseError};
use crate::parsing::exchange_message_type::DeribitMessage;

// THIS IS DEFAULT PARSER, NOT USED ANYMORE

#[derive(Debug, Clone, Deserialize)]
pub struct TradeUpdateMessage {
    pub data: Vec<TradeData>,
}


#[derive(Debug, Clone, Deserialize)]
pub struct TradeData {
    pub timestamp: u64,
    pub price: f32,
    pub amount: f32,
    pub direction: i8,
    pub instrument_name: String,
    pub trade_id: String,
}

#[derive(Debug)]
pub struct TradeDataRef<'a> {
    pub timestamp: u64,
    pub price: f32,
    pub amount: f32,
    pub direction: i8,
    pub instrument_name: &'a str,
    pub trade_id: &'a str,
}

impl MessageParser {
    pub fn parse_trade_update_owned(value: &BorrowedValue) -> Result<DeribitMessage, ParseError> {
        let params = value.get("params")
            .ok_or_else(|| ParseError::MissingField("params".to_string()))?;
        let data = params.get("data")
            .ok_or_else(|| ParseError::MissingField("data".to_string()))?
            .as_array()
            .ok_or_else(|| ParseError::InvalidFormat("data not array".to_string()))?;
        let trades: Result<Vec<TradeData>, ParseError> = data
            .iter()
            .map(Self::parse_trade_data_owned)
            .collect();

        Ok(DeribitMessage::TradeUpdate(TradeUpdateMessage {data: trades?}))
    }

    #[inline]
    fn parse_trade_data_owned(value: &BorrowedValue) -> Result<TradeData, ParseError> {
        Ok(TradeData {
            timestamp: Self::get_u64(value, "timestamp")?,
            price: Self::get_f64(value, "price")? as f32,
            amount: Self::get_f64(value, "amount")? as f32,
            direction: match Self::get_string(value, "direction")?.as_str() {
                "buy" => 1,
                _ => -1,
            },
            instrument_name: Self::get_string(value, "instrument_name")?,
            trade_id: Self::get_string(value, "trade_id")?,
        })
    }
}