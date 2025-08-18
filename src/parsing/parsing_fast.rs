use crate::parsing::ParseError;
use haiku_common::shm_accessor::market_data_type::TradeEvent;
use smallvec::SmallVec;
use std::collections::HashMap;
use crate::parsing::parsing_fast_orderbook::OrderbookResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ChannelType {
    Trades = 0,
    Orderbook = 1,
    Unknown = 2,
}

pub enum FastMarketData {
    Trade(SmallVec<[TradeEvent; 4]>),
    OrderbookUpdate(OrderbookResult)
}


impl FastMarketData {
    #[inline]
    pub fn instrument_idx(&self) -> usize {
        match self {
            FastMarketData::Trade(trade) => trade[0].instrument_idx as usize,
            FastMarketData::OrderbookUpdate(update) => update.instrument_idx,
        }
    }
}


pub struct StreamingParser {
    pub(crate) instrument_map: HashMap<String, usize>,
}

// This need refactoring if it goes in prod, as the code is a bit disgusting and not readable
impl StreamingParser {
    const SUBSCRIPTION_PREFIX: &'static [u8] =
        br#"{"jsonrpc":"2.0","method":"subscription","params":{""#;
    pub(crate) const CHANNEL_PATTERN: &'static [u8] = b"channel";
    const TRADES_PATTERN: &'static [u8] = b"trades.";
    const BOOK_PATTERN: &'static [u8] = b"book.";
    const DATA_PATTERN: &'static [u8] = b"data";
    pub fn new(instrument_map: HashMap<String, usize>) -> Self {
        Self { instrument_map }
    }

    pub fn parse_fast_new(&self, buffer: &[u8]) -> Result<Option<FastMarketData>, ParseError> {
        // Skip common prefix: {'jsonrpc': '2.0', 'method': 'subscription', 'params': {'
        let channel_type = self.detect_channel_type_fast(buffer)?;
        match channel_type {
            ChannelType::Trades => Ok(Some(FastMarketData::Trade(self.parse_trade_fast(buffer)?))),
            ChannelType::Orderbook => Ok(Some(FastMarketData::OrderbookUpdate(self.parse_orderbook_fast(buffer)?))),
            ChannelType::Unknown => Ok(None),
        }
    }

    fn detect_channel_type_fast(&self, buffer: &[u8]) -> Result<ChannelType, ParseError> {
        if buffer.len() < 100 || &buffer[0..52] != Self::SUBSCRIPTION_PREFIX {
            return Ok(ChannelType::Unknown);
        }
        let channel_start = 52 + Self::CHANNEL_PATTERN.len() + 3;
        if &buffer[channel_start..channel_start + 7] == Self::TRADES_PATTERN {
            Ok(ChannelType::Trades)
        } else if &buffer[channel_start..channel_start + 5] == Self::BOOK_PATTERN {
            Ok(ChannelType::Orderbook)
        } else {
            Ok(ChannelType::Unknown)
        }
    }

    #[inline(never)]
    pub fn parse_trade_fast(&self, buffer: &[u8]) -> Result<SmallVec<[TradeEvent; 4]>, ParseError> {
        if buffer.len() < 52 || &buffer[0..52] != Self::SUBSCRIPTION_PREFIX {
            return Err(ParseError::BufferTooShort(buffer.len().to_string()));
        }

        let channel_start = 52 + Self::CHANNEL_PATTERN.len() + 3;

        if buffer.len() < channel_start + 7
            || &buffer[channel_start..channel_start + 7] != Self::TRADES_PATTERN
        {
            return Err(ParseError::BufferTooShort(buffer.len().to_string())); // to modify
        }

        // this should put this just at the "t" of timestamp
        let mut pos = Self::find_data_array_fast(buffer, channel_start)? + 5;
        let mut trades = SmallVec::new();
        let mut original_pos = pos;
        loop {
            if pos + 10 >= buffer.len() || buffer[pos] == 93 {
                // ]
                break;
            }
            if let Some((trade, new_pos)) = self.parse_trade_object_optimized(&buffer[pos..])? {
                trades.push(trade);
                original_pos += new_pos + 3;
                pos = original_pos;
            }
        }
        Ok(trades)
    }

    #[inline]
    pub(crate) fn find_data_array_fast(buffer: &[u8], start: usize) -> Result<usize, ParseError> {
        buffer[start..]
            .windows(Self::DATA_PATTERN.len())
            .position(|w| w == Self::DATA_PATTERN)
            .map(|pos| start + pos + Self::DATA_PATTERN.len())
            .ok_or_else(|| ParseError::InvalidFormat("data not found".to_string()))
    }

    fn parse_trade_object_optimized(
        &self,
        buffer: &[u8],
    ) -> Result<Option<(TradeEvent, usize)>, ParseError> {
        // channel":"trades.ETH-PERPETUAL.raw","data":[{"timestamp":1753469821143,"price":3653.4,"amount":139.0,"direction":"buy","index_price":3654.33,"instrument_name":
        // We start here: timestamp": 1234, "price": 123.45, "amount": 10.0, "direction": "buy", "instrument_name": "ETH-PERPETUAL", "trade_id": "123"}

        // {=123, }=125, "=34, :=58, ,=44, '39
        let mut pos = 0;
        pos = Self::expect_field(buffer, pos, b"timestamp")?;
        let (timestamp, new_pos) = Self::parse_u64(buffer, pos)?;
        pos = new_pos;
        if &buffer[pos + 2..pos + 7] != b"price" {
            return Err(ParseError::FastParserTrade("price".to_string()));
        }

        pos = pos + 9;

        let (price, new_pos) = Self::parse_f64_new_new(buffer, pos)?;
        pos = new_pos + 2;

        if &buffer[pos..pos + 6] != b"amount" {
            return Err(ParseError::FastParserTrade("price".to_string()));
        }
        pos = pos + 8;

        let (amount, new_pos) = Self::parse_f64_new_new(buffer, pos)?;
        pos = new_pos + 2;
        if &buffer[pos..pos + 9] != b"direction" {
            return Err(ParseError::FastParserTrade("price".to_string()));
        }
        pos = pos + 12;
        let direction: u8;
        if &buffer[pos..pos + 3] == b"buy" {
            direction = 1u8;
            pos = pos + 3;
        } else if &buffer[pos..pos + 4] == b"sell" {
            direction = 0u8;
            pos = pos + 4;
        } else {
            return Err(ParseError::FastParserTrade("Direction Unknown".to_string()));
        }
        pos = pos + 3;
        if &buffer[pos..pos + 11] != b"index_price" {
            return Err(ParseError::FastParserTrade("index_price".to_string()));
        }
        pos = pos + 13;
        let (index_price, new_pos) = Self::parse_f64_new(buffer, pos)?;
        pos = new_pos + 2;
        if &buffer[pos..pos + 15] != b"instrument_name" {
            return Err(ParseError::FastParserTrade("instrument_name".to_string()));
        }
        pos = pos + 18;
        let (dir_start, dir_end) = Self::parse_string(buffer, pos)?;
        let instrument_name = std::str::from_utf8(&buffer[dir_start..dir_end])
            .map_err(|_| ParseError::InvalidFormat("Invalid UTF-8".to_string()))?;
        pos = dir_end + 3;
        let instrument_idx = self.instrument_map[instrument_name];
        // let instrument_idx = 1;
        // "trade_seq":187471866,"mark_price":3653.79,"tick_direction":0,"trade_id":"ETH-259727165","contracts":139.0}
        if &buffer[pos..pos + 9] != b"trade_seq" {
            return Err(ParseError::FastParserTrade("instrument_name".to_string()));
        }
        pos = pos + 11;
        let (trade_seq, new_pos) = Self::parse_f64_new(buffer, pos)?;
        pos = new_pos + 2;
        if &buffer[pos..pos + 10] != b"mark_price" {
            return Err(ParseError::FastParserTrade("mark_price".to_string()));
        }
        pos = pos + 12;
        let (mark_price, new_pos) = Self::parse_f64_new(buffer, pos)?;

        pos = new_pos + 2;
        if &buffer[pos..pos + 14] != b"tick_direction" {
            return Err(ParseError::FastParserTrade("tick_direction".to_string()));
        }
        pos = pos + 16;
        let tick_direction: u8 = if buffer[pos] == 48 {
            0u8
        } else if buffer[pos] == 49 {
            1u8
        } else if buffer[pos] == 50 {
            2u8
        } else if buffer[pos] == 51 {
            3u8
        } else {
            return Err(ParseError::FastParserTrade(
                "Invalid tick_direction".to_string(),
            ));
        };
        pos = pos + 3;

        if &buffer[pos..pos + 8] != b"trade_id" {
            return Err(ParseError::FastParserTrade("trade_id".to_string()));
        }
        pos = pos + 11;
        let (dir_start, dir_end) = Self::parse_string(buffer, pos)?;
        let trade_id = std::str::from_utf8(&buffer[dir_start..dir_end])
            .map_err(|_| ParseError::InvalidFormat("Invalid UTF-8".to_string()))?;
        pos = dir_end + 3;

        if &buffer[pos..pos + 9] != b"contracts" {
            return Err(ParseError::FastParserTrade("contracts".to_string()));
        }
        pos = pos + 11;
        let (contracts, new_pos) = Self::parse_f64_new_new(buffer, pos)?;
        if buffer[new_pos] == 125 {
            Ok(Some((
                TradeEvent {
                    instrument_idx: instrument_idx as u8,
                    price: price as f32,
                    size: amount as f32,
                    side: direction,
                    timestamp_ns: timestamp,
                    trade_id: TradeEvent::parse_trade_id(trade_id),
                    padding: [0; 6],
                },
                new_pos + 1,
            )))
        } else {
            Err(ParseError::FastParserTrade("contracts".to_string()))
        }
    }

    #[inline]
    fn expect_field(buffer: &[u8], pos: usize, field: &[u8]) -> Result<usize, ParseError> {
        let pos = Self::skip_whitespace(buffer, pos);
        if pos + field.len() + 3 > buffer.len() {
            return Err(ParseError::InvalidFormat("Field not found".to_string()));
        }

        if &buffer[pos..pos + field.len()] != field || buffer[pos + 1 + field.len()] != 58 {
            println!("expect_field: failed to parse field {:?}", &buffer[pos..pos + 1 + field.len()]);
            return Err(ParseError::InvalidFormat("Field format error".to_string()));
        }

        // We are just after the ": so just before the value
        Ok(pos + 2 + field.len())
    }

    #[inline]
    fn skip_whitespace(buffer: &[u8], mut pos: usize) -> usize {
        while pos < buffer.len() {
            match buffer[pos] {
                32 | 9 | 10 | 13 => pos += 1, // space, tab, newline, carriage return
                _ => break,
            }
        }
        pos
    }


    pub(crate) fn parse_string(buffer: &[u8], pos: usize) -> Result<(usize, usize), ParseError> {
        if pos >= buffer.len() {
            return Err(ParseError::InvalidFormat("Expected string".to_string()));
        }

        let start = pos;
        let mut end = start;
        while end < buffer.len() && buffer[end] != b'"' {
            if buffer[end] == b'\\' {
                end += 2;
            } else {
                end += 1;
            }
        }

        if end >= buffer.len() {
            return Err(ParseError::InvalidFormat("Unterminated string".to_string()));
        }

        Ok((start, end))
    }

    pub(crate) fn parse_u64(buffer: &[u8], pos: usize) -> Result<(u64, usize), ParseError> {
        let pos = Self::skip_whitespace(buffer, pos);

        let start = pos;
        let mut end = start;

        while end < buffer.len() && buffer[end].is_ascii_digit() {
            end += 1;
        }

        if start == end {
            return Err(ParseError::InvalidFormat("Expected number".to_string()));
        }

        let num_str = unsafe { std::str::from_utf8_unchecked(&buffer[start..end]) };
        let val = num_str
            .parse()
            .map_err(|_| ParseError::InvalidFormat("Invalid number".to_string()))?;
        Ok((val, end))
    }

    fn parse_f64(buffer: &[u8], pos: usize) -> Result<(f64, usize), ParseError> {
        let start = pos;
        let mut end = start;

        while end < buffer.len()
            && (buffer[end].is_ascii_digit() || buffer[end] == b'.' || buffer[end] == b'-')
        {
            end += 1;
        }

        if start == end {
            return Err(ParseError::InvalidFormat("Expected number".to_string()));
        }

        // let num_str = std::str::from_utf8(&buffer[start..end]).unwrap();
        let num_str = unsafe { std::str::from_utf8_unchecked(&buffer[start..end]) };
        let val = num_str
            .parse::<f64>()
            .map_err(|_| ParseError::InvalidFormat("Invalid number".to_string()))?;
        Ok((val, end))
    }

    #[inline]
    pub(crate) fn parse_f64_new_new(buffer: &[u8], pos: usize) -> Result<(f64, usize), ParseError> {
        let start = pos;
        let mut end = start;
        let mut integer_part = 0u64;
        let mut decimal_part = 0u64;
        let mut decimal_places = 0u32;
        let mut negative = false;

        if end < buffer.len() && buffer[end] == 45 { // '-'
            negative = true;
            end += 1;
        }

        while end < buffer.len() && buffer[end] >= 48 && buffer[end] <= 57 {
            integer_part = integer_part * 10 + (buffer[end] - 48) as u64;
            end += 1;
        }

        if end < buffer.len() && buffer[end] == 46 {
            end += 1;
            while end < buffer.len() && buffer[end] >= 48 && buffer[end] <= 57 {
                decimal_part = decimal_part * 10 + (buffer[end] - 48) as u64;
                decimal_places += 1;
                end += 1;
            }
        }

        // This part is for the "e" or "E", as sometimes you have price or size with exponent in it...
        let mut exponent = 0i32;
        if end < buffer.len() && (buffer[end] == 101 || buffer[end] == 69) {
            end += 1;
            let mut exp_negative = false;

            if end < buffer.len() && buffer[end] == 45 {
                exp_negative = true;
                end += 1;
            } else if end < buffer.len() && buffer[end] == 43 {
                end += 1;
            }

            while end < buffer.len() && buffer[end] >= 48 && buffer[end] <= 57 {
                exponent = exponent * 10 + (buffer[end] - 48) as i32;
                end += 1;
            }

            if exp_negative {
                exponent = -exponent;
            }
        }

        if start == end || (negative && start + 1 == end) {
            return Err(ParseError::InvalidFormat("Expected number".to_string()));
        }

        let mut result = integer_part as f64;
        if decimal_places > 0 {
            result += (decimal_part as f64) / (10u64.pow(decimal_places) as f64);
        }

        if exponent != 0 {
            result *= 10f64.powi(exponent);
        }

        if negative {
            result = -result;
        }

        Ok((result, end))
    }

    #[inline]
    pub(crate) fn parse_f64_new(buffer: &[u8], pos: usize) -> Result<(f64, usize), ParseError> {
        let start = pos;
        let mut end = start;
        let mut integer_part = 0u64;
        let mut decimal_part = 0u64;
        let mut decimal_places = 0u32;
        let mut negative = false;

        if end < buffer.len() && buffer[end] == 45 {
            // '-'
            negative = true;
            end += 1;
        }

        while end < buffer.len() && buffer[end] >= 48 && buffer[end] <= 57 {
            // 0-9
            integer_part = integer_part * 10 + (buffer[end] - 48) as u64;
            end += 1;
        }

        if end < buffer.len() && buffer[end] == 46 {
            // '.'
            end += 1;
            while end < buffer.len() && buffer[end] >= 48 && buffer[end] <= 57 {
                decimal_part = decimal_part * 10 + (buffer[end] - 48) as u64;
                decimal_places += 1;
                end += 1;
            }
        }

        if start == end || (negative && start + 1 == end) {
            return Err(ParseError::InvalidFormat("Expected number".to_string()));
        }

        let mut result = integer_part as f64;
        if decimal_places > 0 {
            result += (decimal_part as f64) / (10u64.pow(decimal_places) as f64);
        }

        if negative {
            result = -result;
        }

        Ok((result, end))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_streaming_trade_parser() {
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"trades.ETH-PERPETUAL.raw","data":[{"timestamp":1753469821143,"price":3653.4,"amount":139.0,"direction":"buy","index_price":3654.33,"instrument_name":"ETH-PERPETUAL","trade_seq":187471866,"mark_price":3653.79,"tick_direction":0,"trade_id":"ETH-259727165","contracts":139.0}]}}"#;
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"trades.ETH-PERPETUAL.raw","data":[{"timestamp":1753469786907,"price":3652.7,"amount":13830.0,"direction":"sell","index_price":3652.65,"instrument_name":"ETH-PERPETUAL","trade_seq":187471858,"mark_price":3652.16,"tick_direction":0,"trade_id":"ETH-259727150","contracts":13830.0},{"timestamp":1753469786907,"price":3652.7,"amount":29.0,"direction":"sell","index_price":3652.65,"instrument_name":"ETH-PERPETUAL","trade_seq":187471859,"mark_price":3652.16,"tick_direction":1,"trade_id":"ETH-259727151","contracts":29.0}]}}"#;
        let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"trades.ETH-PERPETUAL.raw","data":[{"timestamp":1753688052758,"price":3890.2,"amount":50.0,"direction":"sell","index_price":3889.91,"instrument_name":"ETH-PERPETUAL","trade_seq":187651254,"mark_price":3890.47,"tick_direction":2,"trade_id":"ETH-260011624","contracts":50.0},{"timestamp":1753688052758,"price":3890.2,"amount":7239.0,"direction":"sell","index_price":3889.91,"instrument_name":"ETH-PERPETUAL","trade_seq":187651255,"mark_price":3890.47,"tick_direction":3,"trade_id":"ETH-260011625","contracts":7239.0},{"timestamp":1753688052758,"price":3890.2,"amount":5778.0,"direction":"sell","index_price":3889.91,"instrument_name":"ETH-PERPETUAL","trade_seq":187651256,"mark_price":3890.47,"tick_direction":3,"trade_id":"ETH-260011626","contracts":5778.0}]}}"#;
        let mut instrument_map = HashMap::new();
        instrument_map.insert("ETH-PERPETUAL".to_string(), 1usize);

        let parser = StreamingParser::new(instrument_map);
        let trades = parser.parse_trade_fast(json_data.as_bytes()).unwrap();
        // println!("Trades: {:?}", trades);
        assert_eq!(trades.len(), 2);
        let trade = &trades[0];

        assert_eq!(trade.instrument_idx, 1);
        assert_eq!(trade.price, 3652.7);
        assert_eq!(trade.size, 13830.0);
        assert_eq!(trade.side, 0); // buy
        assert_eq!(trade.timestamp_ns, 1753469786907);
        assert_eq!(trade.trade_id, 259727150); // parsed from "ETH-259123126"
    }

    #[test]
    fn test_order_book_parser() {
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"book.ETH-PERPETUAL.raw","data":{"timestamp":1753616120667,"type":"change","change_id":78324750698,"instrument_name":"ETH-PERPETUAL","bids":[["change",3825.7,132934.0]],"asks":[],"prev_change_id":78324750697}}}"#;
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"book.ETH-PERPETUAL.raw","data":{"timestamp":1753616120667,"type":"change","change_id":78324750698,"instrument_name":"ETH-PERPETUAL","bids":[["change",3825.7,132934.0],["delete",3815.7,1329.0]],"asks":[],"prev_change_id":78324750697}}}"#;
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"book.ETH-PERPETUAL.raw","data":{"timestamp":1753616120667,"type":"change","change_id":78324750698,"instrument_name":"ETH-PERPETUAL","bids":[],"asks":[["change",3825.7,132934.0],["delete",3815.7,1329.0]],"prev_change_id":78324750697}}}"#;
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"book.ETH-PERPETUAL.raw","data":{"timestamp":1753616120667,"type":"change","change_id":78324750698,"instrument_name":"ETH-PERPETUAL","bids":[["new",3825.7,132934.0],["delete",3815.7,1329.0],["change",3825.7,132934.0],["delete",3815.7,1329.0]],"asks":[["change",3825.7,132934.0],["delete",3815.7,1329.0]],"prev_change_id":78324750697}}}"#;
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"book.ETH-PERPETUAL.raw","data":{"timestamp":1753607648212,"type":"snapshot","change_id":78311875036,"instrument_name":"ETH-PERPETUAL","bids":[["new",3770.7,260189.0],["new",3770.65,25004],["new",3770.6,14004],["new",3770.55,23506.0],["new",3770.5,43360.0],["new",3770.45,34993.0],["new",3770.4,2150.0],["new",3770.3,7166.0],["new",3770.2,43435.0],["new",3770.15,12965],["new",3770.05,1196.0],["new",3770.0,70016.0],["new",3769.95,124.0],["new",3769.8,15793.0],["new",3769.75,13111.0],["new",3769.7,23371.0],["new",3769.6,10353.0],["new",3769.55,7226.0],["new",3769.45,1.5e4],["new",3769.4,17962.0],["new",3769.35,69941.0],["new",3769.25,13450.0],["new",3769.2,5381.0],["new",3769.05,11903.0],["new",3769.0,7198.0],["new",3768.85,1223.0],["new",3768.8,7468.0],["new",3768.75,12997.0],["new",3768.6,15288.0],["new",3768.55,11306.0],["new",3768.45,4894.0],["new",3768.4,17839.0],["new",3768.35,51903.0],["new",3768.3,7409.0],["new",3768.2,3721.0],["new",3768.0,35164.0],["new",3767.9,72040.0],["new",3767.85,7409.0],["new",3767.8,259844.0],["new",3767.75,12389.0],["new",3767.7,2.0e4],["new",3767.6,852395.0],["new",3767.55,2867.0],["new",3767.5,2901.0],["new",3767.45,7409.0],["new",3767.3,12258.0],["new",3767.2,3076.0],["new",3767.15,3.0e3],["new",3767.05,7252.0],["new",3767.0,3676.0],["new",3766.95,12252.0],["new",3766.8,20216.0],["new",3766.75,12252.0],["new",3766.7,50295.0],["new",3766.6,7280.0],["new",3766.55,7545.0],["new",3766.5,12608.0],["new",3766.35,3587.0],["new",3766.3,190.0],["new",3766.25,11903.0],["new",3766.2,10.0],["new",3766.05,3779.0],["new",3766.0,104470.0],["new",3765.95,4680.0],["new",3765.9,312044.0],["new",3765.8,2881.0],["new",3765.75,10.0],["new",3765.7,10.0],["new",3765.65,12531.0],["new",3765.6,10921.0],["new",3765.5,19230.0],["new",3765.4,19578.0],["new",3765.35,71736.0],["new",3765.3,239120.0],["new",3765.2,8443.0],["new",3765.15,28144.0],["new",3764.95,397990.0],["new",3764.85,1219723.0],["new",3764.8,136294.0],["new",3764.7,2.5e4],["new",3764.65,20.0],["new",3764.6,11859.0],["new",3764.5,10.0],["new",3764.4,3720.0],["new",3764.35,12411.0],["new",3764.2,5820.0],["new",3764.15,12145.0],["new",3764.1,7254.0],["new",3764.0,19003.0],["new",3763.95,63107.0],["new",3763.8,13115.0],["new",3763.75,23497.0],["new",3763.7,7086.0],["new",3763.6,12349.0],["new",3763.3,7424.0],["new",3763.25,12298.0],["new",3763.05,12280.0],["new",3763.0,1.5e4],["new",3762.9,19438.0],["new",3762.75,12173.0],["new",3762.55,100330.0],["new",3762.5,12307.0],["new",3762.35,11859.0],["new",3762.3,7297.0],["new",3762.2,2.5e4],["new",3762.1,11903.0],["new",3762.0,2628.0],["new",3761.9,7086.0],["new",3761.8,412633.0],["new",3761.7,5.01e4],["new",3761.5,19444.0],["new",3761.25,1046.0],["new",3761.2,3.0e3],["new",3761.15,11994.0],["new",3761.1,7187.0],["new",3761.0,11884.0],["new",3760.8,2.5e4],["new",3760.75,11915.0],["new",3760.7,7112.0],["new",3760.6,11859.0],["new",3760.55,1.125e6],["new",3760.4,398031.0],["new",3760.3,105548.0],["new",3760.2,62115.0],["new",3760.0,24939.0],["new",3759.9,7391.0],["new",3759.85,11964.0],["new",3759.75,438.0],["new",3759.55,845.0],["new",3759.5,19187.0],["new",3759.35,12609.0],["new",3759.1,19206.0],["new",3758.75,11964.0],["new",3758.7,57247.0],["new",3758.5,11964.0],["new",3758.35,12036.0],["new",3758.3,7173.0],["new",3758.2,12173.0],["new",3757.5,7086.0],["new",3757.4,2.5e4],["new",3757.2,5.01e4],["new",3757.15,8.01e4],["new",3757.1,7338.0],["new",3756.95,135250.0],["new",3756.7,7173.0],["new",3756.35,177.0],["new",3756.0,20176.0],["new",3755.9,7267.0],["new",3755.8,78315.0],["new",3755.75,2.5e4],["new",3755.7,5.01e4],["new",3755.5,683870.0],["new",3755.0,792.0],["new",3754.2,5.01e4],["new",3753.9,7422.0],["new",3753.4,8.01e4],["new",3753.1,7390.0],["new",3753.0,148810.0],["new",3752.7,7422.0],["new",3751.5,35.0],["new",3750.95,32.0],["new",3749.65,8.01e4],["new",3747.9,2.5e6],["new",3747.35,153325.0],["new",3747.0,2898.0],["new",3746.25,2090.0],["new",3745.9,8.01e4],["new",3744.75,20101.0],["new",3744.4,5.0],["new",3744.05,230.0],["new",3743.7,229.0],["new",3743.05,60.0],["new",3742.15,8.01e4],["new",3739.9,3.0e6],["new",3739.8,98175.0],["new",3738.8,1.0e4],["new",3738.4,8.01e4],["new",3736.65,6431.0],["new",3736.6,24570.0],["new",3736.45,28221.0],["new",3736.35,34202.0],["new",3736.3,12521.0],["new",3736.25,25123.0],["new",3736.1,1957.0],["new",3735.75,3298.0],["new",3734.8,1.0e4],["new",3734.65,8.01e4],["new",3731.7,300.0],["new",3731.4,297.0],["new",3730.9,8.01e4],["new",3729.1,2667.0],["new",3728.5,98515.0],["new",3727.15,8.01e4],["new",3727.0,600.0],["new",3725.3,55.0],["new",3723.45,8.01e4],["new",3721.0,1856.0],["new",3720.0,500.0],["new",3719.75,8.01e4],["new",3718.35,14523.0],["new",3716.05,8.01e4],["new",3713.85,8756.0],["new",3713.4,102965.0],["new",3713.15,699.0],["new",3712.95,65557.0],["new",3712.75,29680.0],["new",3712.7,7159.0],["new",3712.35,8.01e4],["new",3711.0,1856.0],["new",3710.0,1.7e3],["new",3709.0,40.0],["new",3708.65,8.01e4],["new",3704.95,8.01e4],["new",3701.25,8.01e4],["new",3701.0,1856.0],["new",3700.65,200.0],["new",3.7e3,1.5e3],["new",3699.7,97.0],["new",3699.0,24570.0],["new",3698.9,14387.0],["new",3698.8,28221.0],["new",3698.75,34201.0],["new",3698.7,12521.0],["new",3698.65,25123.0],["new",3698.5,1957.0],["new",3698.35,831.0],["new",3698.2,3298.0],["new",3697.55,8.01e4],["new",3695.0,100.0],["new",3694.45,8658.0],["new",3693.9,379.0],["new",3693.85,8.01e4],["new",3693.7,7790.0],["new",3693.6,29130.0],["new",3693.45,65017.0],["new",3691.1,200.0],["new",3691.0,1856.0],["new",3690.15,8.01e4],["new",3690.0,1.7e3],["new",3687.0,1.0e3],["new",3686.45,8.01e4],["new",3686.4,1.0e3],["new",3685.9,36859.0],["new",3685.1,48565.0],["new",3685.0,1843.0],["new",3682.75,234.0],["new",3681.1,200.0],["new",3681.0,1856.0],["new",3680.0,1.8e3],["new",3674.05,170.0],["new",3670.0,1.8e3],["new",3669.6,2.0e3],["new",3666.1,200.0],["new",3666.0,55002.0],["new",3663.0,5.6e3],["new",3662.25,620.0],["new",3662.0,400.0],["new",3661.0,1856.0],["new",3660.25,14793.0],["new",3660.0,1.6e3],["new",3659.5,11733.0],["new",3658.25,27638.0],["new",3658.0,84131.0],["new",3655.1,200.0],["new",3655.0,2.0e3],["new",3652.75,4.0e4],["new",3651.5,1750.0],["new",3650.5,200.0],["new",3650.3,1607.0],["new",3650.15,2667.0],["new",3650.0,1.5e3],["new",3648.45,297.0],["new",3648.0,1.0e4],["new",3645.25,1945.0],["new",3645.0,113.0],["new",3644.0,1.0e4],["new",3643.0,1.0e4],["new",3641.75,48788.0],["new",3641.65,200.0],["new",3640.0,1.5e3],["new",3635.0,200.0],["new",3632.35,150.0],["new",3632.25,90.0],["new",3631.05,8.4e3],["new",3630.0,1.5e3],["new",3626.0,18130.0],["new",3625.0,2175.0],["new",3622.8,520.0],["new",3621.15,214.0],["new",3620.0,1.6e3],["new",3618.45,361849.0],["new",3615.75,3878.0],["new",3614.1,1.0e3],["new",3612.0,1.0e3],["new",3610.0,1.5e3],["new",3609.0,1.0e5],["new",3607.5,50.0],["new",3603.3,250.0],["new",3.6e3,24674.0],["new",3596.0,17510.0],["new",3594.9,400.0],["new",3593.0,1.0e5],["new",3590.0,1.4e4],["new",3589.5,1.0e4],["new",3588.0,897.0],["new",3585.0,1.0e4],["new",3581.0,1.0e5],["new",3580.0,1001.0],["new",3570.0,1.5e3],["new",3569.6,2.0e3],["new",3562.0,17810.0],["new",3561.05,1.5e3],["new",3560.0,1.5e3],["new",3552.6,2667.0],["new",3550.0,25250.0],["new",3549.8,1764.0],["new",3547.7,3.0e4],["new",3545.9,134.0],["new",3540.0,1.005e5],["new",3537.0,1.0e3],["new",3533.0,2.2e4],["new",3530.0,1360.0],["new",3526.85,6942.0],["new",3522.85,100.0],["new",3520.0,1.05e4],["new",3519.25,5.0e3],["new",3518.0,70360.0],["new",3514.0,2.0e5],["new",3510.0,2.05e4],["new",3505.0,7010.0],["new",3503.0,30.0],["new",3502.0,30.0],["new",3.5e3,88330.0],["new",3499.0,480.0],["new",3496.0,30.0],["new",3495.9,100.0],["new",3495.1,501.0],["new",3493.0,30.0],["new",3492.0,30.0],["new",3491.75,1.9e3],["new",3490.25,20.0],["new",3490.0,1030.0],["new",3486.0,30.0],["new",3485.0,34850.0],["new",3483.0,30.0],["new",3482.0,30.0],["new",3481.0,30.0],["new",3480.25,750.0],["new",3480.0,2.5e3],["new",3479.0,30.0],["new",3476.75,2.5e3],["new",3476.0,30.0],["new",3475.0,5050.0],["new",3474.0,7878.0],["new",3473.0,36361.0],["new",3470.0,1.0e3],["new",3469.0,30.0],["new",3466.0,30.0],["new",3465.0,33740.0],["new",3463.0,30.0],["new",3462.6,1.0e3],["new",3462.0,51930.0],["new",3461.0,30.0],["new",3460.0,47080.0],["new",3456.7,3.0e4],["new",3455.9,100.0],["new",3455.0,2.4e4],["new",3453.0,1.0e4],["new",3450.0,20550.0],["new",3440.0,3.0e3],["new",3435.0,750.0],["new",3430.0,1.0e3],["new",3426.3,3.0e4],["new",3426.15,5.5e3],["new",3425.55,220.0],["new",3425.0,19310.0],["new",3422.65,6.0],["new",3420.5,500.0],["new",3420.0,350005.0],["new",3419.3,1.0e4],["new",3417.4,100.0],["new",3417.3,20.0],["new",3414.15,19997.0],["new",3410.0,344355.0],["new",3407.95,100.0],["new",3402.75,3.0e3],["new",3.4e3,362355.0],["new",3391.35,3.0e4],["new",3390.0,344355.0],["new",3385.0,445.0],["new",3381.0,2.5e3],["new",3380.0,3.49e4],["new",3376.0,33760.0],["new",3374.25,100.0],["new",3370.0,1.0e3],["new",3362.6,1.0e3],["new",3362.0,33620.0],["new",3360.0,344355.0],["new",3356.3,1.0e4],["new",3353.0,3.0e4],["new",3350.0,2.85e4],["new",3345.0,150.0],["new",3340.0,6.1e3],["new",3339.25,8.4e3],["new",3338.1,90.0],["new",3335.0,4168.0],["new",3334.0,4.87e4],["new",3331.5,4258.0],["new",3330.0,1.0e3],["new",3328.6,100.0],["new",3323.0,3.0e4],["new",3320.0,1.0e3],["new",3315.3,2.5e5],["new",3315.0,2.45e4],["new",3310.0,1750.0],["new",3307.0,50.0],["new",3301.15,20.0],["new",3.3e3,7.95e4],["new",3293.0,5.0e4],["new",3280.95,100.0],["new",3277.4,2.5e5],["new",3277.0,150.0],["new",3273.0,3.0e4],["new",3262.0,1.631e5],["new",3260.0,6520.0],["new",3255.0,1.0e5],["new",3250.0,5.69e4],["new",3249.95,343355.0],["new",3231.4,100.0],["new",3221.8,3.0e3],["new",3.2e3,4.25e4],["new",3193.4,20.0],["new",3181.8,100.0],["new",3176.95,2.5e4],["new",3176.25,600.0],["new",3175.0,373355.0],["new",3165.0,150.0],["new",3155.0,9.0e5],["new",3152.3,4148.0],["new",3150.9,100.0],["new",3150.0,389355.0],["new",3139.0,2.5e5],["new",3138.55,300.0],["new",3133.2,100.0],["new",3133.0,174.0],["new",3130.9,100.0],["new",3125.0,343355.0],["new",3120.0,1023777.0],["new",3113.4,20.0],["new",3113.0,33760.0],["new",3104.0,1.0e4],["new",3.1e3,399355.0],["new",3088.9,5.0e4],["new",3079.15,100.0],["new",3070.0,1.0e5],["new",3066.0,3.0e3],["new",3062.0,1.0e5],["new",3061.95,3.0],["new",3055.0,5.0e3],["new",3051.0,1.0e3],["new",3050.0,5.95e4],["new",3030.0,1214850.0],["new",3022.7,60.0],["new",3010.0,9780.0],["new",3.0e3,8.05e4],["new",2993.0,150.0],["new",2991.0,1.0e4],["new",2988.0,2.0e4],["new",2983.7,2.5e3],["new",2980.0,100.0],["new",2979.15,1.0e4],["new",2975.0,500.0],["new",2968.65,20.0],["new",2963.75,2.5e3],["new",2950.0,37050.0],["new",2943.2,2.0e4],["new",2940.0,100.0],["new",2936.9,2.5e3],["new",2936.0,2.0e4],["new",2930.0,300.0],["new",2926.0,2.5e4],["new",2924.05,20.0],["new",2922.9,100.0],["new",2920.0,6.0e3],["new",2918.0,200.0],["new",2917.1,2.5e3],["new",2.9e3,1.26e4],["new",2898.5,5.0e3],["new",2861.0,2.5e4],["new",2851.45,3.0e3],["new",2850.0,2.5e3],["new",2831.0,5.0e3],["new",2826.0,2.5e4],["new",2.8e3,2.1e3],["new",2796.1,300.0],["new",2780.0,100.0],["new",2778.1,1.9e3],["new",2770.0,150.0],["new",2750.0,204750.0],["new",2744.0,20.0],["new",2740.0,100.0],["new",2731.0,5.0e3],["new",2727.4,5.7e4],["new",2726.0,2.5e4],["new",2707.0,1.0e5],["new",2.7e3,5340.0],["new",2688.0,1344.0],["new",2685.0,2.5e4],["new",2681.0,3.0e4],["new",2660.0,100.0],["new",2650.0,2.0e3],["new",2640.5,5.0e3],["new",2625.0,100.0],["new",2621.0,2.5e4],["new",2620.0,100.0],["new",2.6e3,4.2e4],["new",2588.0,1.5e4],["new",2580.0,100.0],["new",2577.5,100.0],["new",2567.0,3.0e3],["new",2550.0,2.0e3],["new",2532.2,100.0],["new",2531.0,2.5e4],["new",2525.0,1.51e4],["new",2.5e3,24590.0],["new",2494.1,1856.0],["new",2460.0,1.5e4],["new",2450.0,2.1e3],["new",2440.0,1.0e3],["new",2403.0,100.0],["new",2390.0,600.0],["new",2350.0,200.0],["new",2252.25,100.0],["new",2250.0,5.0e4],["new",2206.2,2.0e4],["new",2.2e3,712.0],["new",2199.0,12130.0],["new",2188.0,1.0e4],["new",2183.0,1.5e4],["new",2182.1,100.0],["new",2157.0,500.0],["new",2126.1,100.0],["new",2117.0,5.0e4],["new",2115.0,1.0e4],["new",2.1e3,2.1e4],["new",2089.9,100.0],["new",2080.0,678.0],["new",2060.0,42410.0],["new",2050.0,3.0],["new",2039.25,212045.0],["new",2030.0,2030.0],["new",2022.0,1.0e3],["new",2015.0,450.0],["new",2.0e3,2040.0],["new",1993.05,42410.0],["new",1990.0,2.2e3],["new",1972.35,5.0e3],["new",1959.35,5.0e3],["new",1950.0,2150.0],["new",1922.0,1.0e3],["new",1921.3,90.0],["new",1920.0,200.0],["new",1910.0,5.5e5],["new",1885.35,3.0],["new",1825.0,550.0],["new",1822.0,1.0e3],["new",1813.35,4.0e4],["new",1803.3,10.0],["new",1.8e3,1.0],["new",1790.0,1790.0],["new",1740.0,1.5e3],["new",1737.0,6.0],["new",1722.0,1.0e3],["new",1690.0,37.0],["new",1676.0,1.0e3],["new",1622.0,1.0e3],["new",1610.0,1610.0],["new",1598.1,20.0],["new",1588.0,1.0e3],["new",1530.0,300.0],["new",1522.0,1.0e3],["new",1518.0,300.0],["new",1.5e3,310.0],["new",1470.0,1.5e3],["new",1461.0,500.0],["new",1460.0,1.0e5],["new",1450.0,400.0],["new",1438.0,200.0],["new",1422.0,1.0e3],["new",1420.0,500.0],["new",1418.0,500.0],["new",1415.0,300.0],["new",1.4e3,1510.0],["new",1383.2,16262.0],["new",1350.0,2.512e5],["new",1338.65,100.0],["new",1335.0,500.0],["new",1330.0,1.0e3],["new",1322.0,1.0e3],["new",1320.0,1.0e3],["new",1.3e3,3610.0],["new",1272.0,500.0],["new",1250.0,500.0],["new",1222.0,1.0e3],["new",1214.0,350.0],["new",1.2e3,3410.0],["new",1122.0,1.0e3],["new",1.1e3,5010.0],["new",1050.0,2.0e3],["new",1020.0,10.0],["new",1010.0,1.01e4],["new",1.0e3,20.0],["new",999.0,10.0],["new",939.0,30.0],["new",900.0,10.0],["new",800.0,10.0],["new",600.0,57772.0],["new",591.8,40.0],["new",522.0,1.0e3],["new",391.85,50.0],["new",326.1,200.0],["new",256.0,10.0],["new",180.0,1.0],["new",165.0,700.0],["new",155.3,2.0],["new",10.0,100.0]],"asks":[["new",3770.75,43458.0],["new",3771.55,100682.0],["new",3771.7,3658.0],["new",3771.75,12193.0],["new",3771.95,42063.0],["new",3772.0,139327.0],["new",3772.05,32778.0],["new",3772.2,11358.0],["new",3772.25,37063.0],["new",3772.35,2411.0],["new",3772.4,11547.0],["new",3772.45,12173.0],["new",3772.5,26318.0],["new",3772.65,10266.0],["new",3772.7,49392.0],["new",3772.8,3591.0],["new",3773.0,13030.0],["new",3773.15,7145.0],["new",3773.2,3938.0],["new",3773.3,11964.0],["new",3773.4,10367.0],["new",3773.45,12173.0],["new",3773.5,75.0],["new",3773.55,32145.0],["new",3773.6,255161.0],["new",3773.7,17962.0],["new",3773.75,12175.0],["new",3773.8,843920.0],["new",3773.95,7217.0],["new",3774.0,9615.0],["new",3774.05,32050.0],["new",3774.2,15180.0],["new",3774.35,4.0e4],["new",3774.4,11859.0],["new",3774.5,151.0],["new",3774.55,5091.0],["new",3774.7,3.0e3],["new",3774.75,11896.0],["new",3774.8,92.0],["new",3775.0,2.32e4],["new",3775.05,48525.0],["new",3775.1,600.0],["new",3775.15,42938.0],["new",3775.25,500.0],["new",3775.45,99658.0],["new",3775.5,36896.0],["new",3775.6,303648.0],["new",3775.75,12059.0],["new",3775.8,102.0],["new",3775.85,5892.0],["new",3775.9,7175.0],["new",3775.95,12432.0],["new",3776.0,3784.0],["new",3776.15,4720.0],["new",3776.2,16190.0],["new",3776.25,4692.0],["new",3776.3,7231.0],["new",3776.4,38654.0],["new",3776.45,11964.0],["new",3776.5,366.0],["new",3776.55,1.25e5],["new",3776.6,3.6e3],["new",3776.7,11879.0],["new",3776.75,20.0],["new",3776.8,102.0],["new",3776.85,5898.0],["new",3776.9,7259.0],["new",3776.95,20.0],["new",3777.0,382218.0],["new",3777.05,25010.0],["new",3777.1,1.2e6],["new",3777.15,12237.0],["new",3777.2,10172.0],["new",3777.25,16739.0],["new",3777.4,19439.0],["new",3777.5,3558.0],["new",3777.55,11859.0],["new",3777.65,2148.0],["new",3777.8,13728.0],["new",3777.9,11896.0],["new",3777.95,10578.0],["new",3778.0,1.0e4],["new",3778.25,18704.0],["new",3778.3,12696.0],["new",3778.35,2.5e4],["new",3778.55,3569.0],["new",3778.6,17788.0],["new",3778.65,7395.0],["new",3778.8,11859.0],["new",3778.9,3591.0],["new",3778.95,11970.0],["new",3779.05,2209.0],["new",3779.1,7264.0],["new",3779.2,4165.0],["new",3779.4,25028.0],["new",3779.55,8567.0],["new",3779.6,28649.0],["new",3779.65,4894.0],["new",3779.75,500.0],["new",3779.8,12603.0],["new",3779.9,5894.0],["new",3779.95,19647.0],["new",3780.1,267170.0],["new",3780.15,3772.0],["new",3780.2,12562.0],["new",3780.25,4.0e5],["new",3780.35,7252.0],["new",3780.4,60.0],["new",3780.5,3789.0],["new",3780.55,12630.0],["new",3780.7,17859.0],["new",3780.75,7255.0],["new",3780.8,1.25e5],["new",3780.85,415572.0],["new",3781.05,12636.0],["new",3781.15,1007198.0],["new",3781.35,11859.0],["new",3781.55,50675.0],["new",3781.6,12511.0],["new",3781.8,7240.0],["new",3781.95,12511.0],["new",3782.0,37820.0],["new",3782.1,11896.0],["new",3782.2,19577.0],["new",3782.45,12509.0],["new",3782.5,10096.0],["new",3782.55,7086.0],["new",3782.65,36871.0],["new",3782.7,200.0],["new",3782.95,19947.0],["new",3783.15,11896.0],["new",3783.2,2999.0],["new",3783.35,18957.0],["new",3783.6,92689.0],["new",3783.95,19964.0],["new",3784.0,1044.0],["new",3784.1,335.0],["new",3784.2,11859.0],["new",3784.3,5.0e3],["new",3784.35,7325.0],["new",3784.5,13555.0],["new",3784.65,12173.0],["new",3784.75,9425.0],["new",3784.8,160.0],["new",3785.0,37850.0],["new",3785.15,7395.0],["new",3785.35,100470.0],["new",3785.75,691346.0],["new",3787.25,7189.0],["new",3787.4,8.01e4],["new",3787.65,7189.0],["new",3787.7,5.5e3],["new",3788.45,7147.0],["new",3788.65,500.0],["new",3788.85,7241.0],["new",3789.1,1.013e5],["new",3789.2,10989.0],["new",3790.05,7219.0],["new",3790.15,78310.0],["new",3790.85,7189.0],["new",3791.2,8.01e4],["new",3791.65,7228.0],["new",3792.0,37920.0],["new",3792.05,7353.0],["new",3792.4,1.0e3],["new",3792.5,153560.0],["new",3792.85,7425.0],["new",3795.0,118050.0],["new",3795.85,2.5e6],["new",3797.5,15031.0],["new",3797.75,4949.0],["new",3798.0,5.0e4],["new",3798.2,154345.0],["new",3798.35,860.0],["new",3798.8,8.01e4],["new",3799.0,2085.0],["new",3799.7,60.0],["new",3799.75,3128.0],["new",3802.0,38020.0],["new",3802.6,8.01e4],["new",3805.75,97020.0],["new",3806.3,3.0e6],["new",3806.4,8.01e4],["new",3807.95,2.0e5],["new",3808.1,28.0],["new",3809.8,1.0e3],["new",3810.2,8.01e4],["new",3811.0,1145.0],["new",3811.7,792.0],["new",3814.0,8.01e4],["new",3814.35,100.0],["new",3814.45,696.0],["new",3814.55,7092.0],["new",3814.6,63356.0],["new",3814.65,30373.0],["new",3814.7,12481.0],["new",3814.8,3287.0],["new",3814.85,34621.0],["new",3815.0,1.0],["new",3815.05,379.0],["new",3815.65,8665.0],["new",3815.95,1.0e3],["new",3817.05,97045.0],["new",3817.6,70.0],["new",3817.8,8.01e4],["new",3818.0,10641.0],["new",3818.45,295886.0],["new",3819.6,160.0],["new",3819.95,161.0],["new",3820.0,11480.0],["new",3820.15,24571.0],["new",3820.3,25124.0],["new",3820.45,28204.0],["new",3820.65,1957.0],["new",3821.15,833.0],["new",3821.8,14390.0],["new",3824.55,1.0e6],["new",3825.0,5090.0],["new",3826.0,15.0],["new",3826.2,602.0],["new",3827.1,2.0e4],["new",3828.0,7.5e4],["new",3830.0,2540.0],["new",3832.15,97415.0],["new",3834.35,100.0],["new",3834.75,35567.0],["new",3834.95,336457.0],["new",3835.0,5.0e3],["new",3835.05,12557.0],["new",3835.1,65069.0],["new",3835.2,3317.0],["new",3835.3,990.0],["new",3835.5,9139.0],["new",3838.8,550.0],["new",3840.0,500.0],["new",3840.25,38402.0],["new",3840.3,837.0],["new",3840.5,250.0],["new",3840.7,14523.0],["new",3842.6,300.0],["new",3843.0,11982.0],["new",3844.9,150.0],["new",3845.0,1.5e4],["new",3846.2,1.9e3],["new",3846.35,2.4e4],["new",3848.95,1.0e3],["new",3850.0,309350.0],["new",3853.95,3.0e5],["new",3854.35,100.0],["new",3856.95,5.6e3],["new",3857.0,5.0e4],["new",3857.75,24571.0],["new",3857.95,25123.0],["new",3858.0,18780.0],["new",3858.05,28204.0],["new",3858.2,1957.0],["new",3860.0,2980.0],["new",3860.5,50425.0],["new",3863.0,6.0e4],["new",3863.65,25.0],["new",3868.0,3868.0],["new",3869.0,100.0],["new",3869.9,40.0],["new",3870.0,750.0],["new",3877.95,1.0e6],["new",3880.0,500.0],["new",3880.1,1.0e4],["new",3883.0,2.0e3],["new",3883.35,62.0],["new",3884.0,29.0],["new",3884.25,86856.0],["new",3884.5,27809.0],["new",3885.75,11737.0],["new",3886.4,60.0],["new",3886.75,19736.0],["new",3888.0,53888.0],["new",3890.0,2.55e4],["new",3895.0,2.4e4],["new",3899.0,500.0],["new",3.9e3,13230.0],["new",3906.0,3906.0],["new",3906.2,250.0],["new",3910.0,500.0],["new",3912.0,990.0],["new",3915.7,5.0e3],["new",3917.0,2.0e3],["new",3920.0,900.0],["new",3922.35,232.0],["new",3925.0,1.1e4],["new",3930.0,500.0],["new",3931.5,500.0],["new",3932.95,4434.0],["new",3933.25,2.5e3],["new",3935.0,1930.0],["new",3940.0,500.0],["new",3942.0,2.5e4],["new",3942.5,1.5e3],["new",3944.0,11832.0],["new",3944.9,150.0],["new",3949.0,500.0],["new",3950.0,207617.0],["new",3951.55,90.0],["new",3956.0,2.5e5],["new",3960.0,500.0],["new",3963.0,1.0e4],["new",3968.0,250.0],["new",3970.0,1.0e4],["new",3982.3,15.0],["new",3989.0,38850.0],["new",3990.0,15220.0],["new",3992.0,2.5e4],["new",3995.0,1.0e4],["new",3999.0,500.0],["new",3999.5,400.0],["new",4.0e3,311060.0],["new",4001.0,6.0e3],["new",4002.95,4434.0],["new",4006.6,63.0],["new",4014.0,1.0e3],["new",4020.0,2.5e3],["new",4023.0,17832.0],["new",4024.25,1.0e3],["new",4025.0,5.0e3],["new",4033.0,2.95e4],["new",4040.0,15390.0],["new",4048.0,4.048e5],["new",4049.0,7.5e3],["new",4050.0,1.065e5],["new",4051.0,5.0e3],["new",4057.0,3.1e4],["new",4070.0,3.0e3],["new",4071.95,250.0],["new",4075.0,270375.0],["new",4082.5,1.0e3],["new",4086.4,1.0e4],["new",4090.0,1.0e4],["new",4092.25,40.0],["new",4092.5,20.0],["new",4.1e3,2.111e5],["new",4102.0,2.5e3],["new",4111.0,4111.0],["new",4115.0,53470.0],["new",4118.7,250.0],["new",4120.0,60180.0],["new",4135.35,250.0],["new",4150.0,1.0e4],["new",4170.8,250.0],["new",4187.35,887.0],["new",4188.0,100.0],["new",4.2e3,1.568e5],["new",4250.0,1.0e4],["new",4262.6,1.0e4],["new",4272.4,2.0e3],["new",4274.0,5.0e4],["new",4297.9,60.0],["new",4.3e3,1.055e5],["new",4350.0,6.0e3],["new",4385.8,1.0],["new",4.4e3,1075.0],["new",4403.35,250.0],["new",4.5e3,6.0e3],["new",4503.25,60.0],["new",4559.25,1.0],["new",4560.0,1.0e4],["new",4631.0,1.0e3],["new",4650.0,6.0e3],["new",4656.8,1.0],["new",4698.0,2.5e5],["new",4787.3,9575.0],["new",4.8e3,5.5e3],["new",4813.5,250.0],["new",4.9e3,5.0e4],["new",4950.0,1.5e3],["new",5.0e3,1.5e5],["new",5.5e3,1.0e3],["new",5656.15,3.0],["new",6.5e3,500.0],["new",7438.0,1835.0],["new",9850.0,7.5e4],["new",1.0e4,2.5e4],["new",24114.95,100.0],["new",29412.0,450.0]]}}}"#;
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"book.ETH-PERPETUAL.raw","data":{"timestamp":1753686520473,"type":"change","change_id":78450355267,"instrument_name":"ETH-PERPETUAL","bids":[["change",3899.25,21.0]],"asks":[],"prev_change_id":78450355264}}}"#;
        // let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"book.ETH-PERPETUAL.raw","data":{"timestamp":1753686785352,"type":"change","change_id":78451102845,"instrument_name":"ETH-PERPETUAL","bids":[["new",3884.2,1.4e3]],"asks":[],"prev_change_id":78451102844}}}"#;
        let json_data = r#"{"jsonrpc":"2.0","method":"subscription","params":{"channel":"book.ETH-PERPETUAL.raw","data":{"timestamp":1753687679384,"type":"change","change_id":78452945932,"instrument_name":"ETH-PERPETUAL","bids":[],"asks":[["delete",3.9e3,0.0]],"prev_change_id":78452945931}}}"#;
        let mut instrument_map = HashMap::new();
        instrument_map.insert("ETH-PERPETUAL".to_string(), 1usize);

        let parser = StreamingParser::new(instrument_map);
        let trades = parser.parse_orderbook_fast(json_data.as_bytes()).unwrap();
        assert_eq!(3, 2);
    }
}
