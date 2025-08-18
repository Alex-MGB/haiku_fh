use crate::parsing::ParseError;
use crate::parsing::parsing_orderbook::OrderbookLevel;
use crate::parsing::parsing_fast::StreamingParser;
use smallvec::SmallVec;


#[repr(C)]
pub struct OrderbookUpdateDataRaw {
    pub prev_change_id: u64,
    pub bid_updates: SmallVec<[OrderbookLevel; 16]>, // we should monitor the if we have issue with the heap
    pub ask_updates: SmallVec<[OrderbookLevel; 16]>,
    pub flag: u8, // bit 0: has_bids, bit 1: has_asks
}

#[repr(C)]
pub struct OrderbookResult {
    pub change_id: u64,
    pub timestamp: u64,
    pub instrument_idx: usize, 
    pub update_data: OrderbookUpdateDataRaw,
}

impl OrderbookUpdateDataRaw {
    #[inline]
    pub fn new() -> Self {
        Self {
            prev_change_id: 0,
            bid_updates: SmallVec::new(),
            ask_updates: SmallVec::new(),
            flag: 0,
        }
    }

    #[inline]
    pub fn add_bid(&mut self, level: OrderbookLevel) {
        self.bid_updates.push(level);
        self.flag |= 0b01;
    }

    #[inline]
    pub fn add_ask(&mut self, level: OrderbookLevel) {
        self.ask_updates.push(level);
        self.flag |= 0b10;
    }


    #[inline]
    pub fn has_bids(&self) -> bool {
        self.flag & 0b01 != 0
    }

    #[inline]
    pub fn has_asks(&self) -> bool {
        self.flag & 0b10 != 0
    }

    #[inline]
    pub fn set_prev_change_id(&mut self, id: u64) {
        self.prev_change_id = id;
    }
}

impl OrderbookResult {
    #[inline]
    pub fn new(
        change_id: u64,
        timestamp: u64,
        instrument_idx: usize,
        update_data: OrderbookUpdateDataRaw,
    ) -> Self {
        Self {
            change_id,
            timestamp,
            instrument_idx,
            update_data,
        }
    }
}

impl StreamingParser {

    pub fn parse_orderbook_fast(
        &self,
        buffer: &[u8],
    ) -> Result<OrderbookResult, ParseError> {

        let mut pos = Self::find_data_array_fast(buffer, 60 + Self::CHANNEL_PATTERN.len())? + 4;
        if &buffer[pos..pos + 9] != b"timestamp" {
            return Err(ParseError::FastParserTrade("timestamp".to_string()));
        }
        pos = pos + 9 + 2;
        let (timestamp, new_pos) = Self::parse_u64(buffer, pos)?;
        pos = new_pos + 2;

        if &buffer[pos..pos + 4] != b"type" {
            return Err(ParseError::FastParserTrade("type".to_string()));
        }
        pos = pos + 4 + 3;

        let mut data_type;
        if &buffer[pos..pos + 8] == b"snapshot" {
            data_type = 0;
            pos = pos + 8;
        } else if &buffer[pos..pos + 6] == b"change" {
            data_type = 1;
            pos = pos + 6;
        } else {
            return Err(ParseError::FastParserTrade("data type".to_string()));
        }

        pos = pos + 3;
        if &buffer[pos..pos + 9] != b"change_id" {
            return Err(ParseError::FastParserTrade("type".to_string()));
        }
        pos = pos + 9 + 2;
        let (change_id, new_pos) = Self::parse_u64(buffer, pos)?;
        pos = new_pos + 2;
        // {"timestamp":1753607648212,"type":"snapshot","change_id":78311875036,"instrument_name":"ETH-PERPETUAL","bids":[["new",3770.7,260189.0],["new",3770.65,2.5e4],["new",3770.6,1.5e4],["new
        // "bids":[["change",3825.7,132934.0]],"asks":[]

        if &buffer[pos..pos + 15] != b"instrument_name" {
            return Err(ParseError::FastParserTrade("instrument_name".to_string()));
        }
        pos = pos + 18;
        let (dir_start, dir_end) = Self::parse_string(buffer, pos)?;
        let instrument_name = std::str::from_utf8(&buffer[dir_start..dir_end])
            .map_err(|_| ParseError::InvalidFormat("Invalid UTF-8".to_string()))?;
        pos = dir_end + 3;

        if &buffer[pos..pos + 4] != b"bids" {
            return Err(ParseError::FastParserTrade(
                "parse_orderbook_fast bids not found".to_string(),
            ));
        }
        pos = pos + 7;

        let ob_data = if data_type == 0 {
            self.parse_orderbook_snapshot_direct(buffer, pos)?
        } else if data_type == 1 {
            self.parse_order_book_update(buffer, pos)?
        } else {
            return Err(ParseError::FastParserTrade(
                "parse_orderbook_fast: data type not 0 or 1".to_string(),
            ));
        };
        Ok(OrderbookResult::new(change_id, timestamp,self.instrument_map[instrument_name], ob_data))
    }

    fn parse_order_book_update(
        &self,
        buffer: &[u8],
        mut pos: usize,
    ) -> Result<OrderbookUpdateDataRaw, ParseError> {
        let mut ob_data = OrderbookUpdateDataRaw::new();
        loop {
            if let Some((bid, new_pos)) = self.parse_orderbook_entry(buffer, pos) {
                pos = new_pos;
                ob_data.add_bid(bid);
            } else {
                break;
            }
        }

        pos = if ob_data.has_bids() { pos + 2 } else { pos + 3 };
        if &buffer[pos..pos + 4] != b"asks" {
            return Err(ParseError::FastParserTrade(
                "parse_order_book_update: asks not found".to_string(),
            ));
        }
        pos = pos + 7;
        loop {
            if let Some((ask, new_pos)) = self.parse_orderbook_entry(buffer, pos) {
                pos = new_pos;
                ob_data.add_ask(ask);
            } else {
                break;
            }
        }
        pos = if ob_data.has_asks() { pos + 2 } else { pos + 3 };
        if &buffer[pos..pos + 14] != b"prev_change_id" {
            return Err(ParseError::FastParserTrade(
                "parse_order_book_update: prev_change_id".to_string(),
            ));
        }
        pos = pos + 16;
        let (change_id, _) = Self::parse_u64(buffer, pos)?;
        ob_data.set_prev_change_id(change_id);
        Ok(ob_data)
    }

    fn parse_orderbook_snapshot_direct(
        &self,
        buffer: &[u8],
        mut pos: usize,
    ) -> Result<OrderbookUpdateDataRaw, ParseError> {
        // ["new",3770.7,260189.0],["new",3770.65,2.5e4],["new",3770.6,1.5e4],...
        let mut ob_data = OrderbookUpdateDataRaw::new();
        let mut i = 0;
        while i < 10 {
            if let Some((bid, new_pos)) = self.parse_orderbook_entry(buffer, pos) {
                ob_data.add_bid(bid);
                pos = new_pos;
            } else {
                break;
            }
            i = i + 1;
        }
        i = 0;
        pos = self.skip_to_asks_section(&buffer, pos)?;
        while i < 10 {
            if let Some((ask, new_pos)) = self.parse_orderbook_entry(buffer, pos) {
                ob_data.add_ask(ask);
                pos = new_pos;
            } else {
                break;
            }
            i = i + 1;
        }
        Ok(ob_data)
    }

    fn skip_to_asks_section(&self, buffer: &[u8], start_pos: usize) -> Result<usize, ParseError> {
        let pattern = b"\"asks\":[";

        buffer[start_pos..]
            .windows(pattern.len())
            .position(|window| window == pattern)
            .map(|offset| start_pos + offset + pattern.len())
            .ok_or_else(|| ParseError::InvalidFormat("asks section not found".to_string()))
    }

    fn parse_orderbook_entry(
        &self,
        buffer: &[u8],
        mut pos: usize,
    ) -> Option<(OrderbookLevel, usize)> {
        // It should start at the [ before the action so ["new" for example
        if buffer[pos] == 93 {
            None
        } else {
            pos = pos + 2;
            let action;
            if &buffer[pos..pos + 3] == b"new" {
                action = 0;
                pos = pos + 3;
            } else if &buffer[pos..pos + 6] == b"change" {
                action = 1;
                pos = pos + 6;
            } else if &buffer[pos..pos + 6] == b"delete" {
                action = 2;
                pos = pos + 6;
            } else {
                return None;
            }
            pos = pos + 2;
            let (price, new_pos) = match Self::parse_f64_new_new(buffer, pos) {
                Ok(ok) => ok,
                Err(e) => {
                    return None;
                }
            };
            pos = new_pos + 1;
            let (size, new_pos) = match Self::parse_f64_new_new(buffer, pos) {
                Ok(ok) => ok,
                Err(e) => {
                    return None;
                }
            };
            pos = new_pos + 2;
            let level = OrderbookLevel::from_action_number(action, price as f32, size as f32)?;

            Some((level, pos))
        }
    }
}
