use crate::deribit_helper::DeribitError;
use crate::orderbook_management::OrderbookManagerV2;
use crate::parsing::parsing_fast_orderbook::OrderbookResult;
use haiku_common::latency_tracker::LatencyTracker;
use haiku_common::shm_accessor::SHMAccessor;
use haiku_common::shm_accessor::market_data_type::TradeEvent;
use haiku_common::shm_accessor::trade_ring_buffer::TradeRingBuffer;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Duration, Instant};
use tracing::{error, info, warn};


pub async fn shm_writer_task(
    mut fast_trade_rx: mpsc::Receiver<TradeEvent>,
    mut fast_orderbook_rx: mpsc::Receiver<OrderbookResult>,
    mut shutdown_rx: broadcast::Receiver<()>,
    mut shm_writer: SHMAccessor,
    mut trade_buffer: TradeRingBuffer,
    nb_instrument: usize,
) -> Result<(), DeribitError> {

    let mut latency_tracker = LatencyTracker::new(1000);
    let mut ob_manager = Vec::new();

    for _ in 0..nb_instrument {
        ob_manager.push(OrderbookManagerV2::new(10));
    }
    let mut stats_timer = tokio::time::interval(Duration::from_secs(10));

    loop {
        // taskset was failing as the try_recv() wasn't yielding properly to the tokio scheduler so nothing happened (no writing)
        // this is not clean at all
        // we drain the available trades/ob data and then we check for new ones, if we had to drain some channels then we retry immediately
        let mut processed_any = false;

        while let Ok(trade) = fast_trade_rx.try_recv() {
            processed_any = true;
            match trade_buffer.push_trade(trade) {
                Err(e) => error!("Error pushing trade: {:?}", e),
                _ => (),
            }
        }

        while let Ok(orderbook_update) = fast_orderbook_rx.try_recv() {
            processed_any = true;
            let start = Instant::now();
            let flag = orderbook_update.update_data.flag;
            let ob_data = ob_manager[orderbook_update.instrument_idx]
                .apply_update(orderbook_update.update_data, orderbook_update.change_id)
                .unwrap();

            shm_writer
                .write_orderbook_update_consistency_from_idx(
                    orderbook_update.instrument_idx,
                    ob_data,
                    orderbook_update.timestamp,
                    flag,
                )
                .expect("failed to write to SHM");

            latency_tracker.record(start.elapsed());
        }

        if processed_any {
            continue;
        }

        tokio::select! {

            Some(trade) = fast_trade_rx.recv() => {
                match trade_buffer.push_trade(trade) {
                    Err(e) => error!("Error pushing trade: {:?}", e),
                    _ => (),
                }
            }

            Some(orderbook_update) = fast_orderbook_rx.recv() => {
                let start = Instant::now();
                let flag = orderbook_update.update_data.flag;
                let ob_data = ob_manager[orderbook_update.instrument_idx].apply_update(orderbook_update.update_data, orderbook_update.change_id).unwrap();
                shm_writer.write_orderbook_update_consistency_from_idx(
                    orderbook_update.instrument_idx,
                    ob_data,
                    orderbook_update.timestamp,
                    flag,
                ).expect("failed to write to SHM");
                latency_tracker.record(start.elapsed());
            }

            _ = stats_timer.tick() => {
                latency_tracker.print_stats("SHM WRITING");
            }

            _ = shutdown_rx.recv() => {
                return Ok(());
            }
        }
    }
}
