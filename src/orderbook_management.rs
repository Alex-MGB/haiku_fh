use thiserror::Error;
use haiku_common::shm_accessor::market_data_type::OrderbookData;
use crate::parsing::parsing_fast_orderbook::OrderbookUpdateDataRaw;
use crate::parsing::parsing_orderbook::{OrderbookAction, OrderbookLevel, OrderbookSnapshotMessage};

#[derive(Debug, Error)]
pub enum OrderbookError {
    #[error("Sequence gap detected: expected {expected}, got {received}")]
    SequenceGap { expected: u64, received: u64 },

    #[error("Invalid price level: {0}")]
    InvalidLevel(String),

    #[error("Not initialized with snapshot")]
    NotInitialized,
}

#[derive(Debug, Clone, Copy)]
struct PriceLevel {
    price: f32,
    size: f32,
}

#[derive(Debug, Clone)]
pub struct OrderbookManagerV2 {
    instrument: String,
    bids: [PriceLevel; 15], // small buffer if we need to delete some quote which are in our orderbook
    bid_count: u8,
    asks: [PriceLevel; 15],
    ask_count: u8,
    last_change_id: u64,
}

impl OrderbookManagerV2 {
    pub fn new(instrument: String, _max_depth: usize) -> Self {
        Self {
            instrument,
            bids: [PriceLevel { price: 0.0, size: 0.0 }; 15],
            bid_count: 0,
            asks: [PriceLevel { price: 0.0, size: 0.0 }; 15],
            ask_count: 0,
            last_change_id: 0,
        }
    }

    #[inline(always)]
    fn find_price_index_binary(levels: &[PriceLevel], count: u8, price: f32, is_bid: bool) -> Result<usize, usize> {
        if count == 0 {
            return Err(0);
        }

        if is_bid && price > levels[0].price {
            return Err(0); // Insert at front, becomes new best bid
        }
        if !is_bid && price < levels[0].price {
            return Err(0); // Insert at front, becomes new best ask
        }

        let slice = &levels[..count as usize];
        if is_bid {
            // Bids sorted descending
            match slice.binary_search_by(|level| price.partial_cmp(&level.price).unwrap()) {
                Ok(i) => Ok(i),
                Err(i) => Err(i),
            }
        } else {
            // Asks sorted ascending
            match slice.binary_search_by(|level| level.price.partial_cmp(&price).unwrap()) {
                Ok(i) => Ok(i),
                Err(i) => Err(i),
            }
        }
    }

    #[inline(always)]
    fn find_price_index(levels: &[PriceLevel], count: u8, price: f32, is_bid: bool) -> Result<usize, usize> {
        if count == 0 {
            return Err(0);
        }

        // Fast path: check if this becomes new best bid/ask
        if is_bid && price > levels[0].price {
            return Err(0); // Insert at front, becomes new best bid
        }
        if !is_bid && price < levels[0].price {
            return Err(0); // Insert at front, becomes new best ask
        }

        // Check exact match at best price
        if price == levels[0].price {
            return Ok(0);
        }

        let len = count as usize;
        if is_bid {
            for i in 1..len {
                match price.partial_cmp(&levels[i].price).unwrap() {
                    std::cmp::Ordering::Equal => return Ok(i),
                    std::cmp::Ordering::Greater => return Err(i),
                    std::cmp::Ordering::Less => continue,
                }
            }
            Err(len)
        } else {
            for i in 1..len {
                match price.partial_cmp(&levels[i].price).unwrap() {
                    std::cmp::Ordering::Equal => return Ok(i),
                    std::cmp::Ordering::Less => return Err(i),
                    std::cmp::Ordering::Greater => continue,
                }
            }
            Err(len)
        }
    }

    pub fn apply_update(
        &mut self,
        update: OrderbookUpdateDataRaw,
        change_id: u64,
    ) -> Result<OrderbookData, OrderbookError> {
        if update.prev_change_id != self.last_change_id {
            return Err(OrderbookError::SequenceGap {
                    expected: self.last_change_id,
                    received: update.prev_change_id,
                });
            }


        // Process bid updates
        for action in update.bid_updates {

                Self::apply_level_update(
                    &mut self.bids,
                    &mut self.bid_count,
                    action,
                    true,
                )?;

        }

        // Process ask updates
        for action in update.ask_updates {

                Self::apply_level_update(
                    &mut self.asks,
                    &mut self.ask_count,
                    action,
                    false,
                )?;

        }

        self.last_change_id = change_id;
        Ok(self.get_orderbook())
    }

    #[inline(always)]
    fn insert_level(
        levels: &mut [PriceLevel; 15],
        count: &mut u8,
        price: f32,
        size: f32,
        is_bid: bool,
    ) -> Result<(), OrderbookError> {
        if *count >= 15 {
            let worst_idx = 14;
            if is_bid {
                if price <= levels[worst_idx].price {
                    return Ok(());
                }
            } else {
                if price >= levels[worst_idx].price {
                    return Ok(());
                }
            }
            *count = 14;
        }

        match Self::find_price_index_binary(levels, *count, price, is_bid) {
            Ok(idx) => {
                levels[idx].size = size;
            }
            Err(idx) => {
                let current_count = *count as usize;
                if idx < current_count {
                    levels.copy_within(idx..current_count, idx + 1);
                }
                levels[idx] = PriceLevel { price, size };
                *count += 1;
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn remove_level(
        levels: &mut [PriceLevel; 15],
        count: &mut u8,
        price: f32,
        is_bid: bool,
    ) -> Result<(), OrderbookError> {
        if let Ok(idx) = Self::find_price_index_binary(levels, *count, price, is_bid) {
            let current_count = *count as usize;
            if idx + 1 < current_count {
                levels.copy_within(idx + 1..current_count, idx);
            }
            *count -= 1;
        }
        Ok(())
    }

    #[inline(always)]
    fn apply_level_update(
        levels: &mut [PriceLevel; 15],
        count: &mut u8,
        level: OrderbookLevel,
        is_bid: bool,
    ) -> Result<(), OrderbookError> {
        match level.action {
            OrderbookAction::New | OrderbookAction::Change => {
                if level.size > 0.0 {
                    Self::insert_level(levels, count, level.price, level.size, is_bid)?;
                } else {
                    Self::remove_level(levels, count, level.price, is_bid)?;
                }
            }
            OrderbookAction::Delete => {
                Self::remove_level(levels, count, level.price, is_bid)?;
            }
        }
        Ok(())
    }

    #[inline(always)]
    pub fn get_orderbook(&self) -> OrderbookData {
        let mut ob_data = OrderbookData {
            bid_prices: [0.0; 10],
            ask_prices: [0.0; 10],
            bid_sizes: [0.0; 10],
            ask_sizes: [0.0; 10],
        };

        // Copy top 10 bid levels only
        let bid_levels = std::cmp::min(10, self.bid_count as usize);
        for i in 0..bid_levels {
            ob_data.bid_prices[i] = self.bids[i].price;
            ob_data.bid_sizes[i] = self.bids[i].size;
        }

        // Copy top 10 ask levels only
        let ask_levels = std::cmp::min(10, self.ask_count as usize);
        for i in 0..ask_levels {
            ob_data.ask_prices[i] = self.asks[i].price;
            ob_data.ask_sizes[i] = self.asks[i].size;
        }

        ob_data
    }

}