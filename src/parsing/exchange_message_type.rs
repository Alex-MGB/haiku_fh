use crate::parsing::parsing_admin::{AuthMessage, PongMessage, SubscriptionMessage};
use crate::parsing::parsing_orderbook::{OrderbookSnapshotMessage, OrderbookUpdateMessage};
use crate::parsing::parsing_trade::TradeUpdateMessage;

#[derive(Debug, Clone, PartialEq)]
pub enum MessageType {
    Auth,
    Subscription,
    TradeUpdate,
    OrderbookSnapshot,
    OrderbookUpdate,
    Unknown,
    Pong,
}

#[derive(Debug, Clone)]
pub enum DeribitMessage {
    Auth(AuthMessage),
    Subscription(SubscriptionMessage),
    TradeUpdate(TradeUpdateMessage),
    OrderbookSnapshot(OrderbookSnapshotMessage),
    OrderbookUpdate(OrderbookUpdateMessage),
    Unknown,
    Pong(PongMessage),
}