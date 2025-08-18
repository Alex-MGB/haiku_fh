mod config_global;
mod deribit;
mod deribit_helper;
mod parsing;
mod shm_writer;
mod orderbook_management;

use config_global::Config;
use deribit::DeribitConnection;
use haiku_common::metadata::ShmMetadata;
use haiku_common::shm_accessor::SHMAccessor;
use haiku_common::shm_accessor::trade_ring_buffer::TradeRingBuffer;
use tokio::signal;
use tokio::sync::broadcast;
use tracing::{info, warn, error};
use haiku_common::monitoring::logger::StdoutLogger;
use shm_writer::shm_writer_task;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "haiku_fh", about = "Small FH for Deribit")]
struct Args {

    #[arg(long, value_name = "config-fh")]
    config_fh: String,

    #[arg(long, value_name = "config-shm")]
    config_shm: String,
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let cfg = Config::load(&args.config_fh)?;
    let logger = StdoutLogger::new(&cfg.log_path, "fh", &["haiku_fh", "haiku_common"]);
    let metadata = ShmMetadata::load_from_file(&args.config_shm)?;
    let nb_instruments = metadata.max_instruments;

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    let mut connection = DeribitConnection::connect(&cfg.url, metadata.clone_instrument_index(), shutdown_tx).await?;
    let client = connection.client();
    let mut receiver = connection.take_receiver().expect("Failed to get receiver");
    let (fast_trade_rx, fast_orderbook_rx) = connection.take_fast_channels();

    let auth_id = client
        .authenticate(&cfg.key, &cfg.secret)
        .await?;
    let _auth_result = receiver.wait_for_auth_response(auth_id).await?;
    info!("authentication successful to {}", cfg.url);

    let channels = cfg.channels;
    let sub_id = client.subscribe(&channels).await?;
    let _sub_result = receiver.wait_for_subscription_response(sub_id).await?;
    info!("subscribed to channels: {:?}", _sub_result.channels);

    let _connection_handle = connection;
    let mut control_rx = receiver.into_control_rx();

    let shm_writer = SHMAccessor::new("rust_integration", &metadata)
        .expect("failed to open shared memory writer");
    let trade_buffer = TradeRingBuffer::new("rust_integration_trades_buffer", 1000)?;

    println!("spawning shm writer"); // just to know in the terminal all good
    tokio::spawn(shm_writer_task(
        fast_trade_rx,
        fast_orderbook_rx,
        shutdown_rx,
        shm_writer,
        trade_buffer,
        nb_instruments,
    ));


    loop {
        tokio::select! {
            Some(control_msg) = control_rx.recv() => {
                info!("Control message: {:?}", control_msg);
            }

            _ = signal::ctrl_c() => {
                warn!("Shutdown signal received");
                break;
            }
        }
    }
    _connection_handle.shutdown().await?;
    Ok(())
}
