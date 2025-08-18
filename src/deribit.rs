use crate::deribit_helper::{AuthResult, DeribitError, SubscriptionResult};
use crate::parsing::MessageParser;
use crate::parsing::exchange_message_type::DeribitMessage;
use crate::parsing::parsing_fast::{FastMarketData, StreamingParser};
use crate::parsing::parsing_fast_orderbook::OrderbookResult;
use futures::{SinkExt, StreamExt};
use haiku_common::latency_tracker::LatencyTracker;
use haiku_common::monitoring::message_monitor::WebsocketMessageMonitor;
use haiku_common::shm_accessor::market_data_type::TradeEvent;
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};
use tokio_tungstenite::{WebSocketStream, connect_async, tungstenite::protocol::Message};
use tracing::{error, info, warn};

#[derive(Debug)]
pub enum ControlMessage {
    AuthResult {
        id: u64,
        result: Result<AuthResult, DeribitError>,
    },
    SubscriptionResult {
        id: u64,
        result: Result<SubscriptionResult, DeribitError>,
    },
    Error(DeribitError),
}

#[derive(Debug)]
struct ClientCommand {
    msg: String,
    response_tx: oneshot::Sender<Result<Value, DeribitError>>,
}

#[derive(Clone, Debug)]
pub struct DeribitClient {
    command_tx: mpsc::Sender<ClientCommand>,
}

pub struct DeribitReceiver {
    control_rx: mpsc::Receiver<ControlMessage>,
}

pub struct DeribitConnection {
    client: DeribitClient,
    control_rx: Option<mpsc::Receiver<ControlMessage>>,
    receiver: Option<DeribitReceiver>,
    task_handles: Vec<JoinHandle<Result<(), DeribitError>>>,
    shutdown_tx: broadcast::Sender<()>,
    fast_trade_rx: Option<mpsc::Receiver<TradeEvent>>,
    fast_orderbook_rx: Option<mpsc::Receiver<OrderbookResult>>,
}

impl DeribitConnection {
    pub async fn connect(
        url: &str,
        instrument_map: HashMap<String, usize>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Result<Self, DeribitError> {
        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| DeribitError::ConnectionError(e.to_string()))?;

        let (write, read) = ws_stream.split();
        
        let (command_tx, command_rx) = mpsc::channel(100);
        let (parsed_tx, parsed_rx) = mpsc::channel(100);
        let (control_tx, control_rx) = mpsc::channel(100);
        let (fast_trade_tx, fast_trade_rx) = mpsc::channel(1000);
        let (fast_orderbook_tx, fast_orderbook_rx) = mpsc::channel(1000);

        let mut task_handles = Vec::new();
        let streaming_parser = StreamingParser::new(instrument_map);

        let parsed_tx_clone = parsed_tx.clone();
        let mut shutdown_rx_ws = shutdown_tx.subscribe();
        let ws_handle = tokio::spawn(async move {
            Self::websocket_task(
                read,
                write,
                command_rx,
                parsed_tx_clone,
                shutdown_rx_ws,
                fast_trade_tx,
                fast_orderbook_tx,
                streaming_parser,
            )
            .await
        });
        task_handles.push(ws_handle);

        let mut shutdown_rx_router = shutdown_tx.subscribe();

        let router_handle =
            tokio::spawn(
                async move { router_task(parsed_rx, control_tx, shutdown_rx_router).await },
            );
        task_handles.push(router_handle);

        let client = DeribitClient { command_tx };

        Ok(Self {
            client,
            control_rx: None,
            receiver: Some(DeribitReceiver { control_rx }),
            task_handles,
            shutdown_tx,
            fast_trade_rx: Some(fast_trade_rx),
            fast_orderbook_rx: Some(fast_orderbook_rx),
        })
    }

    pub fn take_fast_channels(
        &mut self,
    ) -> (mpsc::Receiver<TradeEvent>, mpsc::Receiver<OrderbookResult>) {
        (
            self.fast_trade_rx
                .take()
                .expect("Fast channels already taken"),
            self.fast_orderbook_rx
                .take()
                .expect("Fast channels already taken"),
        )
    }

    async fn websocket_task(
        mut read: futures::stream::SplitStream<
            WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        >,
        mut write: futures::stream::SplitSink<
            WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
            Message,
        >,
        mut command_rx: mpsc::Receiver<ClientCommand>,
        parsed_tx: mpsc::Sender<DeribitMessage>,
        mut shutdown_rx: broadcast::Receiver<()>,
        fast_trade_tx: mpsc::Sender<TradeEvent>,
        fast_orderbook_tx: mpsc::Sender<OrderbookResult>,
        streaming_parser: StreamingParser,
    ) -> Result<(), DeribitError> {
        let mut parse_buffer = Vec::with_capacity(4096);
        let mut parse_tracker = LatencyTracker::new(1000);
        let mut parse_stats_timer = tokio::time::interval(Duration::from_secs(10));
        let message_monitor = WebsocketMessageMonitor::new();
        let mut ping_timer = tokio::time::interval(Duration::from_secs(480));

        loop {
            tokio::select! {
                ws_msg = read.next() => {
                    match ws_msg {
                        Some(Ok(Message::Text(text))) => {
                            if parse_buffer.capacity() > 8192 {
                                warn!("websocket_task: shrink of parse_buffer");
                                parse_buffer.shrink_to(4096);
                            }
                            message_monitor.record_message();
                            parse_buffer.clear();
                            parse_buffer.extend_from_slice(text.as_bytes());
                            let parse_start = Instant::now();
                            match streaming_parser.parse_fast_new(&mut parse_buffer) {
                                Ok(Some(FastMarketData::Trade(trades))) => {
                                    parse_tracker.record(parse_start.elapsed());
                                    for trade in trades {
                                        let _ = fast_trade_tx.try_send(trade);
                                    }
                                }
                                Ok(Some(FastMarketData::OrderbookUpdate(orderbook))) => {
                                    parse_tracker.record(parse_start.elapsed());
                                    let _ = fast_orderbook_tx.try_send(orderbook);
                                }
                                Ok(None) => {
                                    // slow path for auth/subscription messages
                                    match MessageParser::parse_bytes(&mut parse_buffer) {
                                        Ok(parsed_msg) => {
                                            parse_tracker.record(parse_start.elapsed());
                                            let _ = parsed_tx.try_send(parsed_msg);
                                        }
                                        Err(e) => {
                                            error!("MessageParser error: {:?}", e.to_string());
                                            error!("MessageParser error: {:?}", text);
                                            message_monitor.record_error();}
                                    }
                                }

                                Err(e) => {
                                    error!("streaming_parser error: {:?}", e.to_string());
                                    error!("streaming_parser error: {:?}", text);
                                    message_monitor.record_error(); }
                            }
                        }
                        Some(Err(e)) => {
                            message_monitor.record_error();
                            return Err(DeribitError::ConnectionError(e.to_string()));
                        }

                        None => return Ok(()),

                        _ => {error!("websocket_task UNKNOWN {:?}", ws_msg);
                            continue}

                    }
                }

                cmd = command_rx.recv() => {
                    match cmd {
                        Some(ClientCommand { msg, response_tx }) => {
                            let message = Message::text(msg);
                            if let Err(e) = write.send(message).await {
                                let _ = response_tx.send(Err(DeribitError::ConnectionError(e.to_string())));
                                return Err(DeribitError::ConnectionError(e.to_string()));
                            }
                            let _ = response_tx.send(Ok(Value::Null));
                        }
                        None => return Ok(()),
                    }
                }

                _ = ping_timer.tick() => {
                    let ping_msg = json!({"jsonrpc": "2.0", "method": "public/ping"});
                    if let Err(e) = write.send(Message::text(ping_msg.to_string())).await {
                        error!("websocket_task: Failed to send ping: {}", e);
                        return Err(DeribitError::ConnectionError(e.to_string()));
                    } else {
                        info!("websocket_task: ping has been sent, waiting for pong.");
                    }
                }
                _ = parse_stats_timer.tick() => {
                    parse_tracker.print_stats("PARSE");
                    let messages_stat = message_monitor.get_stats();
                    info!("websocket_task: websocket Message Stats: msg_rates {} | errors {} | last_message_age {}",
                        messages_stat.msg_rate, messages_stat.error_rate, messages_stat.last_message_age);
                }

                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    error!("websocket_task: websocket read timeout");
                    return Err(DeribitError::Timeout);
                }

                _ = shutdown_rx.recv() => {
                    warn!("websocket_task: SHUTDOWN RECEIVED");
                    return Ok(());
                }
            }
        }
    }

    pub fn client(&self) -> DeribitClient {
        self.client.clone()
    }

    pub fn take_receiver(&mut self) -> Option<DeribitReceiver> {
        self.receiver.take()
    }

    pub async fn shutdown(mut self) -> Result<(), DeribitError> {
        let _ = self.shutdown_tx.send(());

        for handle in self.task_handles {
            match handle.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!("Task error during shutdown: {}", e),
                Err(e) => error!("Task join error: {}", e),
            }
        }

        Ok(())
    }
}

async fn router_task(
    mut parsed_rx: mpsc::Receiver<DeribitMessage>,
    control_tx: mpsc::Sender<ControlMessage>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), DeribitError> {
    loop {
        tokio::select! {
            Some(msg) = parsed_rx.recv() => {
                match msg {
                    DeribitMessage::Auth(auth) => {
                        let result = AuthResult {access_token: auth.result.access_token.clone(), expires_in: auth.result.expires_in};
                        let control_msg = ControlMessage::AuthResult {id: auth.id, result: Ok(result)};
                        let _ = control_tx.try_send(control_msg);
                        }
                    DeribitMessage::Subscription(sub) => {
                        let result = SubscriptionResult {channels: sub.result.clone(),success: true};
                        let control_msg = ControlMessage::SubscriptionResult {id: sub.id,result: Ok(result)};
                        let _ = control_tx.try_send(control_msg);
                    }
                    DeribitMessage::Pong(pong) => {
                        info!("router_task: received pong usDiff {} usIn {} usOut {}", pong.us_diff, pong.us_in, pong.us_out);
                    }
                    _ => {}
                    }
            }
            _ = shutdown_rx.recv() => return Ok(()),
        }
    }
}


impl DeribitReceiver {
    pub fn into_control_rx(self) -> mpsc::Receiver<ControlMessage> {
        self.control_rx
    }

    pub async fn wait_for_auth_response(
        &mut self,
        expected_id: u64,
    ) -> Result<AuthResult, DeribitError> {
        let timeout_duration = Duration::from_secs(30);

        tokio::time::timeout(timeout_duration, async {
            while let Some(msg) = self.control_rx.recv().await {
                if let ControlMessage::AuthResult { id, result } = msg {
                    if id == expected_id {
                        return result;
                    }
                }
            }
            Err(DeribitError::ChannelClosed)
        })
        .await
        .map_err(|_| DeribitError::Timeout)?
    }

    pub async fn wait_for_subscription_response(
        &mut self,
        expected_id: u64,
    ) -> Result<SubscriptionResult, DeribitError> {
        let timeout_duration = Duration::from_secs(10);

        tokio::time::timeout(timeout_duration, async {
            while let Some(msg) = self.control_rx.recv().await {
                info!("Subscription message answer: {:#?}", msg);
                if let ControlMessage::SubscriptionResult { id, result } = msg {
                    if id == expected_id {
                        return result;
                    }
                }
            }
            Err(DeribitError::ChannelClosed)
        })
        .await
        .map_err(|_| DeribitError::Timeout)?
    }
}

impl DeribitClient {
    pub async fn authenticate(&self, api_key: &str, api_secret: &str) -> Result<u64, DeribitError> {
        let uuid = 1;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": uuid,
            "method": "public/auth",
            "params": {
                "grant_type": "client_credentials",
                "client_id": api_key,
                "client_secret": api_secret
            }
        });
        self.send_command(msg.to_string()).await?;
        Ok(1)
    }

    pub async fn subscribe(&self, channels: &[String]) -> Result<u64, DeribitError> {
        let id = 2;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "public/subscribe",
            "params": { "channels": channels },
        });
        self.send_command(msg.to_string()).await?;
        Ok(id)
    }

    async fn send_command(&self, msg: String) -> Result<(), DeribitError> {
        let (response_tx, _response_rx) = oneshot::channel();
        let command = ClientCommand { msg, response_tx };

        self.command_tx
            .send(command)
            .await
            .map_err(|_| DeribitError::ConnectionError("Command channel closed".into()))?;

        Ok(())
    }
}
