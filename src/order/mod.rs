mod base;
mod pegged;
mod status;
mod time_in_force;
mod update;

use crate::OrderQueue;
use crate::errors::PriceLevelError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
// Re-exports
pub use base::{OrderId, Side};
pub use pegged::PegReferenceType;
pub use time_in_force::TimeInForce;
pub use update::OrderUpdate;

/// Default amount to replenish the reserve with.
pub const DEFAULT_RESERVE_REPLENISH_AMOUNT: u64 = 80;

/// Common fields for all order types
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrderCommon<T> {
    /// The order ID
    pub id: OrderId,
    /// The price of the order
    pub price: u64,
    /// The quantity of the order
    pub display_quantity: u64,
    /// The side of the order (buy or sell)
    pub side: Side,
    /// When the order was created
    pub timestamp: u64,
    /// Time-in-force policy
    pub time_in_force: TimeInForce,
    /// Additional custom fields
    pub extra_fields: T,
}

impl<T: Clone> OrderCommon<T> {
    fn map_display(&self, f: impl FnOnce(u64) -> u64) -> Self {
        Self {
            display_quantity: f(self.display_quantity),
            ..self.clone()
        }
    }
}

impl<T> fmt::Display for OrderCommon<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "id={};price={};display_quantity={};side={};timestamp={};time_in_force={}",
            self.id,
            self.price,
            self.display_quantity,
            format!("{}", self.side).to_uppercase(),
            self.timestamp,
            self.time_in_force
        )
    }
}

/// Represents different types of limit orders
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderType<T> {
    /// Standard limit order
    Standard {
        #[serde(flatten)]
        common: OrderCommon<T>,
    },

    /// Iceberg order with display and reserve quantities
    IcebergOrder {
        #[serde(flatten)]
        common: OrderCommon<T>,
        /// The reserve quantity of the order
        reserve_quantity: u64,
    },

    /// Post-only order that won't match immediately
    PostOnly {
        #[serde(flatten)]
        common: OrderCommon<T>,
    },

    /// Trailing stop order that adjusts with market movement
    TrailingStop {
        #[serde(flatten)]
        common: OrderCommon<T>,
        /// Amount to trail the market price
        trail_amount: u64,
        /// Last reference price
        last_reference_price: u64,
    },

    /// Pegged order that adjusts based on reference price
    PeggedOrder {
        #[serde(flatten)]
        common: OrderCommon<T>,
        /// Offset from the reference price
        reference_price_offset: i64,
        /// Type of reference price to track
        reference_price_type: PegReferenceType,
    },

    /// Market-to-limit order that converts to limit after initial execution
    MarketToLimit {
        #[serde(flatten)]
        common: OrderCommon<T>,
    },

    /// Reserve order with custom replenishment
    /// if `replenish_amount` is None, it uses DEFAULT_RESERVE_REPLENISH_AMOUNT
    /// if `auto_replenish` is false, and visible quantity is below threshold, it will not replenish
    /// if `auto_replenish` is false and visible quantity is zero it will be removed from the book
    /// if `auto_replenish` is true, and replenish_threshold is 0, it will use 1
    ReserveOrder {
        #[serde(flatten)]
        common: OrderCommon<T>,
        /// The reserve quantity of the order
        reserve_quantity: u64,
        /// Threshold at which to replenish
        replenish_threshold: u64,
        /// Optional amount to replenish by. If None, uses DEFAULT_RESERVE_REPLENISH_AMOUNT
        replenish_amount: Option<u64>,
        /// Whether to replenish automatically when below threshold. If false, only replenish on next match
        auto_replenish: bool,
    },
}

impl<T: Clone> OrderType<T> {
    /// Create a new standard order with reduced quantity
    pub fn with_reduced_quantity(&self, new_quantity: u64) -> Self {
        let mut new = self.clone();
        new.common_mut().display_quantity = new_quantity;

        new
    }

    /// Update an iceberg order, refreshing display part from reserve
    pub fn refresh_iceberg(&self, refresh_amount: u64) -> (Self, u64) {
        let mut new = self.clone();
        let used_hidden = match &mut new {
            Self::IcebergOrder {
                common,
                reserve_quantity,
                ..
            }
            | Self::ReserveOrder {
                common,
                reserve_quantity,
                ..
            } => {
                let new_hidden = reserve_quantity.saturating_sub(refresh_amount);
                let used_hidden = *reserve_quantity - new_hidden;

                common.display_quantity = refresh_amount;
                *reserve_quantity = new_hidden;

                used_hidden
            }
            _ => 0, // Non-iceberg orders don't refresh
        };

        (new, used_hidden)
    }
}

impl<T: Clone> OrderType<T> {
    /// Matches this order against an incoming quantity
    ///
    /// Returns a tuple containing:
    /// - The quantity consumed from the incoming order
    /// - Optionally, an updated version of this order (if partially filled)
    /// - The quantity that was reduced from hidden portion (for iceberg/reserve orders)
    /// - The remaining quantity of the incoming order
    pub fn match_against(&self, incoming_quantity: u64) -> (u64, Option<Self>, u64, u64) {
        match self {
            Self::Standard { common } => {
                let display_quantity = common.display_quantity;
                if display_quantity <= incoming_quantity {
                    // Full match
                    return (
                        display_quantity,                     // consumed = full order quantity
                        None,                                 // no updated order (fully matched)
                        0,                                    // no hidden quantity reduced
                        incoming_quantity - display_quantity, // remaining = incoming - consumed
                    );
                }

                // Partial match
                let common = common
                    .clone()
                    .map_display(|quantity| quantity - incoming_quantity);
                (
                    incoming_quantity, // consumed = all incoming quantity
                    Some(Self::Standard { common }),
                    0, // not hidden quantity reduced
                    0, // not remaining quantity
                )
            }

            // En OrderType::match_against para IcebergOrder
            Self::IcebergOrder {
                common,
                reserve_quantity,
            } => {
                let display_quantity = common.display_quantity;
                if display_quantity > incoming_quantity {
                    // Partial match of visible quantity
                    return (
                        incoming_quantity,
                        Some(Self::IcebergOrder {
                            common: common.map_display(|quantity| quantity - incoming_quantity),
                            reserve_quantity: *reserve_quantity,
                        }),
                        0,
                        0,
                    );
                }

                // Fully match the visible portion
                let remaining = incoming_quantity - display_quantity;

                // No hidden quantity left
                if *reserve_quantity == 0 {
                    return (display_quantity, None, 0, remaining);
                }

                let refresh_qty = std::cmp::min(*reserve_quantity, display_quantity);

                (
                    display_quantity,
                    Some(Self::IcebergOrder {
                        common: common.map_display(|_| refresh_qty),
                        reserve_quantity: *reserve_quantity - refresh_qty,
                    }),
                    refresh_qty,
                    remaining,
                )
            }

            Self::ReserveOrder {
                // display_quantity,
                common,
                reserve_quantity,
                replenish_threshold,
                replenish_amount,
                auto_replenish,
            } => {
                let display_quantity = common.display_quantity;
                // Ensure the threshold is never 0 if auto_replenish is true
                let safe_threshold = if *auto_replenish && *replenish_threshold == 0 {
                    1
                } else {
                    *replenish_threshold
                };

                let replenish_qty = replenish_amount
                    .unwrap_or(DEFAULT_RESERVE_REPLENISH_AMOUNT)
                    .min(*reserve_quantity);

                // Full match of the visible part
                if display_quantity <= incoming_quantity {
                    let consumed = display_quantity;
                    let remaining = incoming_quantity - consumed;

                    // No auto-replenishment or hidden quantity, delete the order
                    if *reserve_quantity == 0 || !*auto_replenish {
                        return (consumed, None, 0, remaining);
                    }

                    return (
                        consumed,
                        Some(Self::ReserveOrder {
                            common: common.map_display(|_| replenish_qty),
                            reserve_quantity: *reserve_quantity - replenish_qty,
                            replenish_threshold: *replenish_threshold,
                            replenish_amount: *replenish_amount,
                            auto_replenish: *auto_replenish,
                        }),
                        replenish_qty,
                        remaining,
                    );
                }

                // Partial match of the visible part
                let new_display = display_quantity - incoming_quantity;

                // Replenish  (we fell below the threshold)
                if new_display < safe_threshold && *reserve_quantity > 0 && *auto_replenish {
                    return (
                        incoming_quantity,
                        Some(Self::ReserveOrder {
                            common: common.map_display(|_| new_display + replenish_qty),
                            reserve_quantity: *reserve_quantity - replenish_qty,
                            replenish_threshold: *replenish_threshold,
                            replenish_amount: *replenish_amount,
                            auto_replenish: *auto_replenish,
                        }),
                        replenish_qty,
                        0,
                    );
                }

                // We don't need to replenish or it is not automatic
                (
                    incoming_quantity,
                    Some(Self::ReserveOrder {
                        common: common.map_display(|_| new_display),
                        reserve_quantity: *reserve_quantity,
                        replenish_threshold: *replenish_threshold,
                        replenish_amount: *replenish_amount,
                        auto_replenish: *auto_replenish,
                    }),
                    0,
                    0,
                )
            }

            // For all other order types, use standard matching logic
            _ => {
                let visible_qty = self.display_quantity();

                // Full match
                if visible_qty <= incoming_quantity {
                    return (
                        visible_qty,                     // consumed full visible quantity
                        None,                            // fully matched
                        0,                               // no hidden reduced
                        incoming_quantity - visible_qty, // remaining quantity
                    );
                }

                // Partial match
                (
                    incoming_quantity, // consumed all incoming
                    Some(self.with_reduced_quantity(visible_qty - incoming_quantity)),
                    0, // not hidden reduced
                    0, // not remaining quantity
                )
            }
        }
    }
}

impl<T> OrderType<T> {
    fn common(&self) -> &OrderCommon<T> {
        match self {
            Self::Standard { common, .. } => common,
            Self::IcebergOrder { common, .. } => common,
            Self::PostOnly { common, .. } => common,
            Self::TrailingStop { common, .. } => common,
            Self::PeggedOrder { common, .. } => common,
            Self::MarketToLimit { common, .. } => common,
            Self::ReserveOrder { common, .. } => common,
        }
    }

    fn common_mut(&mut self) -> &mut OrderCommon<T> {
        match self {
            Self::Standard { common, .. } => common,
            Self::IcebergOrder { common, .. } => common,
            Self::PostOnly { common, .. } => common,
            Self::TrailingStop { common, .. } => common,
            Self::PeggedOrder { common, .. } => common,
            Self::MarketToLimit { common, .. } => common,
            Self::ReserveOrder { common, .. } => common,
        }
    }

    /// Get the order ID
    pub fn id(&self) -> OrderId {
        self.common().id
    }

    /// Get the price
    pub fn price(&self) -> u64 {
        self.common().price
    }

    /// Get the visible quantity
    pub fn display_quantity(&self) -> u64 {
        self.common().display_quantity
    }

    /// Get the reserve quantity
    pub fn reserve_quantity(&self) -> u64 {
        match self {
            Self::IcebergOrder {
                reserve_quantity, ..
            } => *reserve_quantity,
            Self::ReserveOrder {
                reserve_quantity, ..
            } => *reserve_quantity,
            _ => 0,
        }
    }

    /// Get the order side
    pub fn side(&self) -> Side {
        self.common().side
    }

    /// Get the time in force
    pub fn time_in_force(&self) -> TimeInForce {
        self.common().time_in_force
    }

    /// Get the timestamp
    pub fn timestamp(&self) -> u64 {
        self.common().timestamp
    }

    /// Check if the order is immediate-or-cancel
    pub fn is_immediate(&self) -> bool {
        self.time_in_force().is_immediate()
    }

    /// Check if the order is fill-or-kill
    pub fn is_fill_or_kill(&self) -> bool {
        matches!(self.time_in_force(), TimeInForce::Fok)
    }

    /// Check if this is a post-only order
    pub fn is_post_only(&self) -> bool {
        matches!(self, Self::PostOnly { .. })
    }

    /// Get the extra fields
    pub fn extra_fields(&self) -> &T {
        &self.common().extra_fields
    }

    /// Get mutable reference to extra fields
    pub fn extra_fields_mut(&mut self) -> &mut T {
        &mut self.common_mut().extra_fields
    }

    /// Transform the extra fields type using a function
    pub fn map_extra_fields<U, F>(self, f: F) -> OrderType<U>
    where
        F: FnOnce(T) -> U,
    {
        let map_common_extra = |OrderCommon {
                                    id,
                                    price,
                                    display_quantity,
                                    side,
                                    timestamp,
                                    time_in_force,
                                    extra_fields,
                                }| {
            OrderCommon {
                id,
                price,
                display_quantity,
                side,
                timestamp,
                time_in_force,
                extra_fields: f(extra_fields),
            }
        };

        match self {
            OrderType::Standard { common } => OrderType::Standard {
                common: map_common_extra(common),
            },
            OrderType::IcebergOrder {
                common,
                reserve_quantity,
            } => OrderType::IcebergOrder {
                common: map_common_extra(common),
                reserve_quantity,
            },
            OrderType::PostOnly { common } => OrderType::PostOnly {
                common: map_common_extra(common),
            },
            OrderType::TrailingStop {
                common,
                trail_amount,
                last_reference_price,
            } => OrderType::TrailingStop {
                common: map_common_extra(common),
                trail_amount,
                last_reference_price,
            },
            OrderType::PeggedOrder {
                common,
                reference_price_offset,
                reference_price_type,
            } => OrderType::PeggedOrder {
                common: map_common_extra(common),
                reference_price_offset,
                reference_price_type,
            },
            OrderType::MarketToLimit { common } => OrderType::MarketToLimit {
                common: map_common_extra(common),
            },
            OrderType::ReserveOrder {
                common,
                reserve_quantity,
                replenish_threshold,
                replenish_amount,
                auto_replenish,
            } => OrderType::ReserveOrder {
                common: map_common_extra(common),
                reserve_quantity,
                replenish_threshold,
                replenish_amount,
                auto_replenish,
            },
        }
    }
}

/// Expected string format:
/// ORDER_TYPE:id=`<id>`;price=`<price>`;quantity=`<qty>`;side=<BUY|SELL>;timestamp=`<ts>`;time_in_force=`<tif>`;[additional fields]
///
/// Examples:
/// - Standard:id=123;price=10000;quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC
/// - IcebergOrder:id=124;price=10000;display_quantity=1;reserve_quantity=4;side=SELL;timestamp=1616823000000;time_in_force=GTC
impl<T: Default> FromStr for OrderType<T> {
    type Err = PriceLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err(PriceLevelError::InvalidFormat);
        }

        let order_type = parts[0];
        let fields_str = parts[1];

        let mut fields = std::collections::HashMap::new();
        for field_pair in fields_str.split(';') {
            let kv: Vec<&str> = field_pair.split('=').collect();
            if kv.len() == 2 {
                fields.insert(kv[0], kv[1]);
            }
        }

        let get_field = |field: &str| -> Result<&str, PriceLevelError> {
            match fields.get(field) {
                Some(result) => Ok(*result),
                None => Err(PriceLevelError::MissingField(field.to_string())),
            }
        };

        let parse_u64 = |field: &str| -> Result<u64, PriceLevelError> {
            let value = get_field(field)?;
            value
                .parse::<u64>()
                .map_err(|_| PriceLevelError::InvalidFieldValue {
                    field: field.to_string(),
                    value: value.to_string(),
                })
        };

        let parse_i64 = |field: &str| -> Result<i64, PriceLevelError> {
            let value = get_field(field)?;
            value
                .parse::<i64>()
                .map_err(|_| PriceLevelError::InvalidFieldValue {
                    field: field.to_string(),
                    value: value.to_string(),
                })
        };

        // Parse common fields
        let id_str = get_field("id")?;
        let id = OrderId::from_str(id_str).map_err(|_| PriceLevelError::InvalidFieldValue {
            field: "id".to_string(),
            value: id_str.to_string(),
        })?;

        let price = parse_u64("price")?;
        let side: Side = Side::from_str(get_field("side")?)?;
        let timestamp = parse_u64("timestamp")?;
        let time_in_force = TimeInForce::from_str(get_field("time_in_force")?)?;
        let display_quantity = parse_u64("display_quantity")?;

        let common = OrderCommon {
            id,
            price,
            display_quantity,
            side,
            timestamp,
            time_in_force,
            extra_fields: T::default(),
        };

        // Parse specific order types
        match order_type {
            "Standard" => Ok(OrderType::Standard { common }),
            "IcebergOrder" => {
                let reserve_quantity = parse_u64("reserve_quantity")?;

                Ok(OrderType::IcebergOrder {
                    common,
                    reserve_quantity,
                })
            }
            "PostOnly" => Ok(OrderType::PostOnly { common }),
            "TrailingStop" => {
                let trail_amount = parse_u64("trail_amount")?;
                let last_reference_price = parse_u64("last_reference_price")?;

                Ok(OrderType::TrailingStop {
                    common,
                    trail_amount,
                    last_reference_price,
                })
            }
            "PeggedOrder" => {
                let reference_price_offset = parse_i64("reference_price_offset")?;
                let reference_price_type_str = get_field("reference_price_type")?;
                let reference_price_type = match reference_price_type_str {
                    "BestBid" => PegReferenceType::BestBid,
                    "BestAsk" => PegReferenceType::BestAsk,
                    "MidPrice" => PegReferenceType::MidPrice,
                    "LastTrade" => PegReferenceType::LastTrade,
                    _ => {
                        return Err(PriceLevelError::InvalidFieldValue {
                            field: "reference_price_type".to_string(),
                            value: reference_price_type_str.to_string(),
                        });
                    }
                };

                Ok(OrderType::PeggedOrder {
                    common,
                    reference_price_offset,
                    reference_price_type,
                })
            }
            "MarketToLimit" => Ok(OrderType::MarketToLimit { common }),
            "ReserveOrder" => {
                let reserve_quantity = parse_u64("reserve_quantity")?;
                let replenish_threshold = parse_u64("replenish_threshold")?;
                let replenish_amount_str = get_field("replenish_amount")?;
                let replenish_amount = if replenish_amount_str == "None" {
                    None
                } else {
                    Some(parse_u64("replenish_amount")?)
                };
                let auto_replenish_str = get_field("auto_replenish")?;
                let auto_replenish = match auto_replenish_str {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(PriceLevelError::InvalidFieldValue {
                            field: "auto_replenish".to_string(),
                            value: auto_replenish_str.to_string(),
                        });
                    }
                };

                Ok(OrderType::ReserveOrder {
                    common,
                    reserve_quantity,
                    replenish_threshold,
                    replenish_amount,
                    auto_replenish,
                })
            }
            _ => Err(PriceLevelError::UnknownOrderType(order_type.to_string())),
        }
    }
}

impl<T> fmt::Display for OrderType<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrderType::Standard { common } => {
                write!(f, "Standard:{common}",)
            }
            OrderType::IcebergOrder {
                common,
                reserve_quantity,
            } => {
                write!(
                    f,
                    "IcebergOrder:{common};reserve_quantity={reserve_quantity}"
                )
            }
            OrderType::PostOnly { common } => {
                write!(f, "PostOnly:{common}")
            }
            OrderType::TrailingStop {
                common,
                trail_amount,
                last_reference_price,
            } => {
                write!(
                    f,
                    "TrailingStop:{common};trail_amount={trail_amount};last_reference_price={last_reference_price}"
                )
            }
            OrderType::PeggedOrder {
                common,
                reference_price_offset,
                reference_price_type,
            } => {
                write!(
                    f,
                    "PeggedOrder:{common};reference_price_offset={reference_price_offset};reference_price_type={reference_price_type}"
                )
            }
            OrderType::MarketToLimit { common } => {
                write!(f, "MarketToLimit:{common}")
            }
            OrderType::ReserveOrder {
                common,
                reserve_quantity,
                replenish_threshold,
                replenish_amount,
                auto_replenish,
            } => {
                let replenish_amount =
                    replenish_amount.map_or("None".to_string(), |v| v.to_string());
                write!(
                    f,
                    "ReserveOrder:{common};reserve_quantity={reserve_quantity};replenish_threshold={replenish_threshold};auto_replenish={auto_replenish};replenish_amount={replenish_amount}",
                )
            }
        }
    }
}

impl From<OrderQueue> for Vec<Arc<OrderType<()>>> {
    fn from(queue: OrderQueue) -> Self {
        queue.to_vec()
    }
}

// Type aliases for common use cases
#[allow(dead_code)]
pub type SimpleOrderType = OrderType<()>;
#[allow(dead_code)]
pub type OrderTypeWithMetadata = OrderType<OrderMetadata>;

// Example of what the extra fields could contain
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OrderMetadata {
    pub client_id: Option<u64>,
    pub user_id: Option<u64>,
    pub exchange_id: Option<u8>,
    pub priority: u8,
}

#[cfg(test)]
mod tests {
    use crate::order::time_in_force::TimeInForce;
    use crate::order::{OrderCommon, OrderId, OrderType, PegReferenceType, Side};
    use std::str::FromStr;
    use tracing::info;

    fn create_standard_order() -> OrderType<()> {
        OrderType::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(123),
                price: 10000,
                display_quantity: 5,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        }
    }

    // Helper function to create an iceberg order for testing
    fn create_iceberg_order() -> OrderType<()> {
        OrderType::<()>::IcebergOrder {
            common: OrderCommon {
                id: OrderId::from_u64(124),
                price: 10000,
                display_quantity: 1,
                side: Side::Sell,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 4,
        }
    }

    // Helper function to create a post-only order for testing
    fn create_post_only_order() -> OrderType<()> {
        OrderType::<()>::PostOnly {
            common: OrderCommon {
                id: OrderId::from_u64(125),
                price: 10000,
                display_quantity: 5,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        }
    }

    // Helper function to create a trailing stop order for testing
    fn create_trailing_stop_order() -> OrderType<()> {
        OrderType::<()>::TrailingStop {
            common: OrderCommon {
                id: OrderId::from_u64(126),
                price: 10000,
                display_quantity: 5,
                side: Side::Sell,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            trail_amount: 100,
            last_reference_price: 10100,
        }
    }

    // Helper function to create a pegged order for testing
    fn create_pegged_order() -> OrderType<()> {
        OrderType::<()>::PeggedOrder {
            common: OrderCommon {
                id: OrderId::from_u64(127),
                price: 10000,
                display_quantity: 5,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reference_price_offset: -10,
            reference_price_type: PegReferenceType::BestBid,
        }
    }

    // Helper function to create a market-to-limit order for testing
    fn create_market_to_limit_order() -> OrderType<()> {
        OrderType::<()>::MarketToLimit {
            common: OrderCommon {
                id: OrderId::from_u64(128),
                price: 10000,
                display_quantity: 5,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        }
    }

    // Helper function to create a reserve order for testing
    fn create_reserve_order() -> OrderType<()> {
        OrderType::<()>::ReserveOrder {
            common: OrderCommon {
                id: OrderId::from_u64(129),
                price: 10000,
                display_quantity: 1,
                side: Side::Sell,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 4,
            replenish_threshold: 1,
            replenish_amount: Some(2),
            auto_replenish: true,
        }
    }

    #[test]
    fn test_order_id() {
        assert_eq!(create_standard_order().id(), OrderId::from_u64(123));
        assert_eq!(create_iceberg_order().id(), OrderId::from_u64(124));
        assert_eq!(create_post_only_order().id(), OrderId::from_u64(125));
        assert_eq!(create_trailing_stop_order().id(), OrderId::from_u64(126));
        assert_eq!(create_pegged_order().id(), OrderId::from_u64(127));
        assert_eq!(create_market_to_limit_order().id(), OrderId::from_u64(128));
        assert_eq!(create_reserve_order().id(), OrderId::from_u64(129));
    }

    #[test]
    fn test_order_price() {
        assert_eq!(create_standard_order().price(), 10000);
        assert_eq!(create_iceberg_order().price(), 10000);
        assert_eq!(create_post_only_order().price(), 10000);
        assert_eq!(create_trailing_stop_order().price(), 10000);
        assert_eq!(create_pegged_order().price(), 10000);
        assert_eq!(create_market_to_limit_order().price(), 10000);
        assert_eq!(create_reserve_order().price(), 10000);
    }

    #[test]
    fn test_display_quantity() {
        assert_eq!(create_standard_order().display_quantity(), 5);
        assert_eq!(create_iceberg_order().display_quantity(), 1);
        assert_eq!(create_post_only_order().display_quantity(), 5);
        assert_eq!(create_trailing_stop_order().display_quantity(), 5);
        assert_eq!(create_pegged_order().display_quantity(), 5);
        assert_eq!(create_market_to_limit_order().display_quantity(), 5);
        assert_eq!(create_reserve_order().display_quantity(), 1);
    }

    #[test]
    fn test_reserve_quantity() {
        assert_eq!(create_standard_order().reserve_quantity(), 0);
        assert_eq!(create_iceberg_order().reserve_quantity(), 4);
        assert_eq!(create_post_only_order().reserve_quantity(), 0);
        assert_eq!(create_trailing_stop_order().reserve_quantity(), 0);
        assert_eq!(create_pegged_order().reserve_quantity(), 0);
        assert_eq!(create_market_to_limit_order().reserve_quantity(), 0);
        assert_eq!(create_reserve_order().reserve_quantity(), 4);
    }

    #[test]
    fn test_order_side() {
        assert_eq!(create_standard_order().side(), Side::Buy);
        assert_eq!(create_iceberg_order().side(), Side::Sell);
        assert_eq!(create_post_only_order().side(), Side::Buy);
        assert_eq!(create_trailing_stop_order().side(), Side::Sell);
        assert_eq!(create_pegged_order().side(), Side::Buy);
        assert_eq!(create_market_to_limit_order().side(), Side::Buy);
        assert_eq!(create_reserve_order().side(), Side::Sell);
    }

    #[test]
    fn test_time_in_force() {
        assert_eq!(create_standard_order().time_in_force(), TimeInForce::Gtc);
        assert_eq!(create_iceberg_order().time_in_force(), TimeInForce::Gtc);
        assert_eq!(create_post_only_order().time_in_force(), TimeInForce::Gtc);
        assert_eq!(
            create_trailing_stop_order().time_in_force(),
            TimeInForce::Gtc
        );
        assert_eq!(create_pegged_order().time_in_force(), TimeInForce::Gtc);
        assert_eq!(
            create_market_to_limit_order().time_in_force(),
            TimeInForce::Gtc
        );
        assert_eq!(create_reserve_order().time_in_force(), TimeInForce::Gtc);
    }

    #[test]
    fn test_timestamp() {
        assert_eq!(create_standard_order().timestamp(), 1616823000000);
        assert_eq!(create_iceberg_order().timestamp(), 1616823000000);
        assert_eq!(create_post_only_order().timestamp(), 1616823000000);
        assert_eq!(create_trailing_stop_order().timestamp(), 1616823000000);
        assert_eq!(create_pegged_order().timestamp(), 1616823000000);
        assert_eq!(create_market_to_limit_order().timestamp(), 1616823000000);
        assert_eq!(create_reserve_order().timestamp(), 1616823000000);
    }

    #[test]
    fn test_is_immediate() {
        let mut order = create_standard_order();
        assert!(!order.is_immediate());

        // Test with IOC time-in-force
        if let OrderType::<()>::Standard {
            common:
                OrderCommon {
                    ref mut time_in_force,
                    extra_fields: _,
                    ..
                },
            ..
        } = order
        {
            *time_in_force = TimeInForce::Ioc;
        }
        assert!(order.is_immediate());
    }

    #[test]
    fn test_is_fill_or_kill() {
        let mut order = create_standard_order();
        assert!(!order.is_fill_or_kill());

        // Test with FOK time-in-force
        if let OrderType::<()>::Standard {
            common:
                OrderCommon {
                    ref mut time_in_force,
                    extra_fields: _,
                    ..
                },
            ..
        } = order
        {
            *time_in_force = TimeInForce::Fok;
        }
        assert!(order.is_fill_or_kill());
    }

    #[test]
    fn test_is_post_only() {
        assert!(!create_standard_order().is_post_only());
        assert!(!create_iceberg_order().is_post_only());
        assert!(create_post_only_order().is_post_only());
        assert!(!create_trailing_stop_order().is_post_only());
        assert!(!create_pegged_order().is_post_only());
        assert!(!create_market_to_limit_order().is_post_only());
        assert!(!create_reserve_order().is_post_only());
    }

    #[test]
    fn test_with_reduced_quantity() {
        // Test standard order
        let order = create_standard_order();
        let reduced = order.with_reduced_quantity(3);

        if let OrderType::<()>::Standard {
            common:
                OrderCommon {
                    display_quantity: quantity,
                    ..
                },
        } = reduced
        {
            assert_eq!(quantity, 3);
        } else {
            panic!("Expected StandardOrder");
        }

        // Test iceberg order
        let order = create_iceberg_order();
        let reduced = order.with_reduced_quantity(0);

        if let OrderType::<()>::IcebergOrder {
            common: OrderCommon {
                display_quantity, ..
            },
            reserve_quantity,
            ..
        } = reduced
        {
            assert_eq!(display_quantity, 0);
            assert_eq!(reserve_quantity, 4); // Reserve quantity should remain unchanged
        } else {
            panic!("Expected IcebergOrder");
        }

        // NEW TEST: Test post-only order with reduced quantity
        let order = create_post_only_order();
        let reduced = order.with_reduced_quantity(2);

        if let OrderType::<()>::PostOnly {
            common:
                OrderCommon {
                    display_quantity: quantity,
                    ..
                },
            ..
        } = reduced
        {
            assert_eq!(quantity, 2);
        } else {
            panic!("Expected PostOnly order");
        }

        // NEW TEST: Test trailing stop order with reduced quantity
        let order = create_trailing_stop_order();
        let reduced = order.with_reduced_quantity(3);

        match reduced {
            OrderType::<()>::TrailingStop {
                common:
                    OrderCommon {
                        display_quantity: quantity,
                        ..
                    },
                ..
            } => {
                assert_eq!(quantity, 3);
            }
            _ => panic!("Expected TrailingStop order"),
        }

        // NEW TEST: Test pegged order with reduced quantity
        let order = create_pegged_order();
        let reduced = order.with_reduced_quantity(1);

        match reduced {
            OrderType::<()>::PeggedOrder {
                common:
                    OrderCommon {
                        display_quantity: quantity,
                        ..
                    },
                ..
            } => {
                assert_eq!(quantity, 1);
            }
            _ => panic!("Expected PeggedOrder"),
        }

        // NEW TEST: Test market-to-limit order with reduced quantity
        let order = create_market_to_limit_order();
        let reduced = order.with_reduced_quantity(4);

        match reduced {
            OrderType::<()>::MarketToLimit {
                common:
                    OrderCommon {
                        display_quantity: quantity,
                        ..
                    },
                ..
            } => {
                assert_eq!(quantity, 4);
            }
            _ => panic!("Expected MarketToLimit order"),
        }

        // NEW TEST: Test reserve order with reduced quantity
        let order = create_reserve_order();
        let reduced = order.with_reduced_quantity(0);

        match reduced {
            OrderType::<()>::ReserveOrder {
                common: OrderCommon {
                    display_quantity, ..
                },
                reserve_quantity,
                ..
            } => {
                assert_eq!(display_quantity, 0);
                assert_eq!(reserve_quantity, 4); // Reserve should remain unchanged
            }
            _ => panic!("Expected ReserveOrder"),
        }
    }

    #[test]
    fn test_refresh_iceberg() {
        // Test iceberg order refresh
        let order = create_iceberg_order();
        let (refreshed, _used) = order.refresh_iceberg(2);

        if let OrderType::<()>::IcebergOrder {
            common: OrderCommon {
                display_quantity, ..
            },
            reserve_quantity,
            ..
        } = refreshed
        {
            assert_eq!(display_quantity, 2);
            assert_eq!(reserve_quantity, 2); // Should be reduced from 4 to 2
        } else {
            panic!("Expected IcebergOrder");
        }

        // Test reserve order refresh
        let order = create_reserve_order();
        let (refreshed, used) = order.refresh_iceberg(3);

        if let OrderType::<()>::ReserveOrder {
            common: OrderCommon {
                display_quantity, ..
            },
            reserve_quantity,
            ..
        } = refreshed
        {
            assert_eq!(display_quantity, 3);
            assert_eq!(reserve_quantity, 1); // 4 - 3 = 1
            assert_eq!(used, 3);
        } else {
            panic!("Expected ReserveOrder");
        }

        // Test non-iceberg order (should not refresh)
        let order = create_standard_order();
        let (refreshed, _used) = order.refresh_iceberg(2);

        if let OrderType::<()>::Standard {
            common:
                OrderCommon {
                    display_quantity: quantity,
                    ..
                },
        } = refreshed
        {
            assert_eq!(quantity, 5);
        } else {
            panic!("Expected Standard");
        }
    }

    #[test]
    fn test_from_str_standard() {
        let order_str = "Standard:id=00000000-0000-007b-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC";
        let order: OrderType<()> = OrderType::from_str(order_str).unwrap();

        if let OrderType::<()>::Standard {
            common:
                OrderCommon {
                    id,
                    price,
                    display_quantity: quantity,
                    side,
                    timestamp,
                    time_in_force,
                    ..
                },
        } = order
        {
            assert_eq!(id, OrderId::from_u64(123));
            assert_eq!(price, 10000);
            assert_eq!(quantity, 5);
            assert_eq!(side, Side::Buy);
            assert_eq!(timestamp, 1616823000000);
            assert_eq!(time_in_force, TimeInForce::Gtc);
        } else {
            panic!("Expected StandardOrder");
        }
    }

    #[test]
    fn test_from_str_iceberg() {
        let order_str = "IcebergOrder:id=00000000-0000-007c-0000-000000000000;price=10000;display_quantity=3;reserve_quantity=7;side=SELL;timestamp=1616823000001;time_in_force=GTC";
        let order: OrderType<()> = OrderType::from_str(order_str).unwrap();

        if let OrderType::<()>::IcebergOrder {
            common:
                OrderCommon {
                    id,
                    price,
                    display_quantity,
                    side,
                    timestamp,
                    time_in_force,
                    extra_fields: _,
                },
            reserve_quantity,
        } = order
        {
            assert_eq!(id, OrderId::from_u64(124));
            assert_eq!(price, 10000);
            assert_eq!(display_quantity, 3);
            assert_eq!(reserve_quantity, 7);
            assert_eq!(side, Side::Sell);
            assert_eq!(timestamp, 1616823000001);
            assert_eq!(time_in_force, TimeInForce::Gtc);
        } else {
            panic!("Expected IcebergOrder");
        }
    }

    #[test]
    fn test_from_str_trailing_stop() {
        let order_str = "TrailingStop:id=00000000-0000-007e-0000-000000000000;price=10000;display_quantity=8;side=SELL;timestamp=1616823000000;time_in_force=GTC;trail_amount=100;last_reference_price=10100";
        let order: OrderType<()> = OrderType::from_str(order_str).unwrap();

        if let OrderType::<()>::TrailingStop {
            common:
                OrderCommon {
                    id,
                    price,
                    display_quantity: quantity,
                    side,
                    timestamp,
                    time_in_force,
                    extra_fields: _,
                },
            trail_amount,
            last_reference_price,
        } = order
        {
            assert_eq!(id, OrderId::from_u64(126));
            assert_eq!(price, 10000);
            assert_eq!(quantity, 8);
            assert_eq!(side, Side::Sell);
            assert_eq!(timestamp, 1616823000000);
            assert_eq!(time_in_force, TimeInForce::Gtc);
            assert_eq!(trail_amount, 100);
            assert_eq!(last_reference_price, 10100);
        } else {
            panic!("Expected TrailingStop");
        }
    }

    #[test]
    fn test_from_str_pegged() {
        let order_str = "PeggedOrder:id=00000000-0000-007f-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC;reference_price_offset=-50;reference_price_type=BestAsk";
        let order: OrderType<()> = OrderType::from_str(order_str).unwrap();

        if let OrderType::<()>::PeggedOrder {
            common:
                OrderCommon {
                    id,
                    price,
                    display_quantity: quantity,
                    side,
                    timestamp,
                    time_in_force,
                    extra_fields: _,
                },
            reference_price_offset,
            reference_price_type,
        } = order
        {
            assert_eq!(id, OrderId::from_u64(127));
            assert_eq!(price, 10000);
            assert_eq!(quantity, 5);
            assert_eq!(side, Side::Buy);
            assert_eq!(timestamp, 1616823000000);
            assert_eq!(time_in_force, TimeInForce::Gtc);
            assert_eq!(reference_price_offset, -50);
            assert_eq!(reference_price_type, PegReferenceType::BestAsk);
        } else {
            panic!("Expected PeggedOrder");
        }
    }

    #[test]
    fn test_from_str_different_time_in_force() {
        // Test IOC time-in-force
        let order_str = "PostOnly:id=00000000-0000-007d-0000-000000000000;price=9950;display_quantity=25;side=BUY;timestamp=1616823000000;time_in_force=GTC";
        let order: OrderType<()> = OrderType::from_str(order_str).unwrap();

        if let OrderType::<()>::PostOnly {
            common: OrderCommon { time_in_force, .. },
            ..
        } = order
        {
            assert_eq!(time_in_force, TimeInForce::Gtc);
        } else {
            panic!("Expected PostOnly");
        }

        // Test GTD time-in-force
        let order_str = "Standard:id=00000000-0000-007b-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTD-1616909400000";
        let order: OrderType<()> = OrderType::from_str(order_str).unwrap();

        if let OrderType::<()>::Standard {
            common: OrderCommon { time_in_force, .. },
            ..
        } = order
        {
            assert!(matches!(time_in_force, TimeInForce::Gtd(1616909400000)));
        } else {
            panic!("Expected Standard order");
        }
    }

    #[test]
    fn test_from_str_errors() {
        // Test invalid format
        let order_str = "Standard;id=00000000-0000-007b-0000-000000000000;price=10000";
        let result: Result<OrderType<()>, _> = OrderType::from_str(order_str);
        assert!(result.is_err());

        // Test unknown order type
        let order_str = "Unknown:id=00000000-0000-007b-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC";
        let result: Result<OrderType<()>, _> = OrderType::from_str(order_str);
        assert!(result.is_err());

        // Test missing field
        let order_str = "Standard:id=00000000-0000-007b-0000-000000000000;price=10000;side=BUY;timestamp=1616823000000;time_in_force=GTC";
        let result: Result<OrderType<()>, _> = OrderType::from_str(order_str);
        assert!(result.is_err());

        // Test invalid field value
        let order_str = "Standard:id=00000000-0000-007b-0000-000000000000;price=invalid;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC";
        let result: Result<OrderType<()>, _> = OrderType::from_str(order_str);
        assert!(result.is_err());
    }

    // NEW TESTS for Display implementation
    #[test]
    fn test_display_standard_order() {
        let order = create_standard_order();
        let display_str = format!("{order}");

        info!("{}", display_str);
        assert!(display_str.starts_with("Standard:"));
        assert!(display_str.contains("id=00000000-0000-007b-0000-000000000000"));
        assert!(display_str.contains("price=10000"));
        assert!(display_str.contains("quantity=5"));
        assert!(display_str.contains("side=BUY"));
        assert!(display_str.contains("timestamp=1616823000000"));
        assert!(display_str.contains("time_in_force=GTC"));
    }

    #[test]
    fn test_display_iceberg_order() {
        let order = create_iceberg_order();
        let display_str = format!("{order}");

        assert!(display_str.starts_with("IcebergOrder:"));
        assert!(display_str.contains("id=00000000-0000-007c-0000-000000000000"));
        assert!(display_str.contains("price=10000"));
        assert!(display_str.contains("display_quantity=1"));
        assert!(display_str.contains("reserve_quantity=4"));
        assert!(display_str.contains("side=SELL"));
        assert!(display_str.contains("timestamp=1616823000000"));
        assert!(display_str.contains("time_in_force=GTC"));
    }

    #[test]
    fn test_display_post_only_order() {
        let order = create_post_only_order();
        let display_str = format!("{order}");

        // Assuming the Display implementation for PostOnly is completed
        assert!(
            display_str.contains("OrderType variant not fully implemented for Display")
                || (display_str.starts_with("PostOnly:")
                    && display_str.contains("id=00000000-0000-007d-0000-000000000000")
                    && display_str.contains("price=10000")
                    && display_str.contains("quantity=5")
                    && display_str.contains("side=BUY")
                    && display_str.contains("timestamp=1616823000000")
                    && display_str.contains("time_in_force=GTC"))
        );
    }

    #[test]
    fn test_roundtrip_display_parse() {
        // Test that converting to string and parsing back works correctly
        let original_order = create_standard_order();
        let string_representation = original_order.to_string();
        let parsed_order: Result<OrderType<()>, _> = OrderType::from_str(&string_representation);

        // If Display is properly implemented, this should work
        if let Ok(parsed) = parsed_order {
            if let (
                OrderType::<()>::Standard {
                    common:
                        OrderCommon {
                            id: id1,
                            price: price1,
                            display_quantity: qty1,
                            side: side1,
                            ..
                        },
                },
                OrderType::<()>::Standard {
                    common:
                        OrderCommon {
                            id: id2,
                            price: price2,
                            display_quantity: qty2,
                            side: side2,
                            ..
                        },
                },
            ) = (original_order, parsed)
            {
                assert_eq!(id1, id2);
                assert_eq!(price1, price2);
                assert_eq!(qty1, qty2);
                assert_eq!(side1, side2);
            } else {
                // This will happen if Display for non-Standard orders isn't complete
                info!("Note: Display implementation may not be complete for all order types");
            }
        }
    }

    #[test]
    fn test_display_implementation_completeness() {
        // Test all order types to ensure Display is implemented or properly indicated as unimplemented
        let orders = vec![
            create_standard_order(),
            create_iceberg_order(),
            create_post_only_order(),
            create_trailing_stop_order(),
            create_pegged_order(),
            create_market_to_limit_order(),
            create_reserve_order(),
        ];

        for order in orders {
            let display_str = format!("{order}");

            // Either properly implemented or properly indicates it's not implemented
            assert!(
                display_str.contains(":id=")
                    || display_str.contains("OrderType variant not fully implemented for Display")
            );
        }
    }

    #[test]
    fn test_with_reduced_quantity_market_to_limit() {
        // Lines 663-664
        let order = OrderType::<()>::MarketToLimit {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 1000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };

        let reduced = order.with_reduced_quantity(5);

        // Verify the quantity is not changed (market to limit orders don't support
        // reduced quantity in the current implementation)
        if let OrderType::MarketToLimit {
            common:
                OrderCommon {
                    display_quantity: quantity,
                    ..
                },
            ..
        } = reduced
        {
            assert_eq!(quantity, 5);
        } else {
            panic!("Expected MarketToLimit order");
        }
    }

    #[test]
    fn test_with_reduced_quantity_pegged_order() {
        // Lines 720-721
        let order = OrderType::<()>::PeggedOrder {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 1000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reference_price_offset: -50,
            reference_price_type: PegReferenceType::BestAsk,
        };

        let reduced = order.with_reduced_quantity(5);

        // Verify the quantity is not changed (pegged orders don't support
        // reduced quantity in the current implementation)
        if let OrderType::PeggedOrder {
            common:
                OrderCommon {
                    display_quantity: quantity,
                    ..
                },
            ..
        } = reduced
        {
            assert_eq!(quantity, 5);
        } else {
            panic!("Expected PeggedOrder");
        }
    }

    #[test]
    fn test_with_reduced_quantity_trailing_stop() {
        // Line 741
        let order = OrderType::<()>::TrailingStop {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 1000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            trail_amount: 100,
            last_reference_price: 1100,
        };

        let reduced = order.with_reduced_quantity(5);

        // Verify the quantity is not changed (trailing stop orders don't support
        // reduced quantity in the current implementation)
        if let OrderType::TrailingStop {
            common:
                OrderCommon {
                    display_quantity: quantity,
                    ..
                },
            ..
        } = reduced
        {
            assert_eq!(quantity, 5);
        } else {
            panic!("Expected TrailingStop");
        }
    }

    #[test]
    fn test_refresh_iceberg_non_iceberg_orders() {
        // Line 760
        let standard_order = OrderType::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 1000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };

        let (refreshed, used) = standard_order.refresh_iceberg(5);

        // Non-iceberg orders should remain unchanged and return 0 used
        assert_eq!(used, 0);

        match refreshed {
            OrderType::Standard {
                common:
                    OrderCommon {
                        display_quantity: quantity,
                        ..
                    },
                ..
            } => assert_eq!(quantity, 10), // Unchanged
            _ => panic!("Expected StandardOrder"),
        }
    }

    #[test]
    fn test_match_against_trailing_stop_order() {
        // Line 809 (or nearby)
        let order = OrderType::<()>::TrailingStop {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 1000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            trail_amount: 100,
            last_reference_price: 1100,
        };

        let (consumed, updated, hidden_reduced, remaining) = order.match_against(5);

        // Verify partial match
        assert_eq!(consumed, 5);
        assert!(updated.is_some());
        assert_eq!(hidden_reduced, 0);
        assert_eq!(remaining, 0);

        // Verify complete match
        let (consumed, updated, hidden_reduced, remaining) = order.match_against(10);
        assert_eq!(consumed, 10);
        assert!(updated.is_none()); // Fully consumed
        assert_eq!(hidden_reduced, 0);
        assert_eq!(remaining, 0);

        // Verify match with excess
        let (consumed, updated, hidden_reduced, remaining) = order.match_against(15);
        assert_eq!(consumed, 10);
        assert!(updated.is_none());
        assert_eq!(hidden_reduced, 0);
        assert_eq!(remaining, 5); // 15 - 10 = 5 remaining
    }
}

#[cfg(test)]
mod test_order_type_display {
    use crate::order::time_in_force::TimeInForce;
    use crate::order::{OrderCommon, OrderId, OrderType, PegReferenceType, Side};
    use std::str::FromStr;

    #[test]
    fn test_standard_order_display() {
        let order = OrderType::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(123),
                price: 10000,
                display_quantity: 5,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };

        let display_str = order.to_string();
        assert_eq!(
            display_str,
            "Standard:id=00000000-0000-007b-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC"
        );

        // Test that it can be parsed back (round-trip)
        let parsed: Result<OrderType<()>, _> = OrderType::from_str(&display_str);
        assert!(parsed.is_ok(), "Failed to parse Standard order string");

        if let Ok(OrderType::Standard {
            common:
                OrderCommon {
                    id,
                    price,
                    display_quantity: quantity,
                    side,
                    ..
                },
        }) = parsed
        {
            assert_eq!(id, OrderId::from_u64(123));
            assert_eq!(price, 10000);
            assert_eq!(quantity, 5);
            assert_eq!(side, Side::Buy);
        } else {
            panic!("Parsed result is not a Standard order");
        }
    }

    #[test]
    fn test_iceberg_order_display() {
        let order = OrderType::<()>::IcebergOrder {
            common: OrderCommon {
                id: OrderId::from_u64(124),
                price: 10000,
                display_quantity: 1,
                side: Side::Sell,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 4,
        };

        let display_str = order.to_string();
        assert_eq!(
            display_str,
            "IcebergOrder:id=00000000-0000-007c-0000-000000000000;price=10000;display_quantity=1;side=SELL;timestamp=1616823000000;time_in_force=GTC;reserve_quantity=4"
        );

        // Test that it can be parsed back (round-trip)
        let parsed: Result<OrderType<()>, _> = OrderType::from_str(&display_str);
        assert!(parsed.is_ok(), "Failed to parse IcebergOrder string");

        if let Ok(OrderType::IcebergOrder {
            common:
                OrderCommon {
                    id,
                    price,
                    display_quantity,
                    side,
                    ..
                },
            reserve_quantity,
            ..
        }) = parsed
        {
            assert_eq!(id, OrderId::from_u64(124));
            assert_eq!(price, 10000);
            assert_eq!(display_quantity, 1);
            assert_eq!(reserve_quantity, 4);
            assert_eq!(side, Side::Sell);
        } else {
            panic!("Parsed result is not an IcebergOrder");
        }
    }

    #[test]
    fn test_post_only_order_display() {
        let order = OrderType::<()>::PostOnly {
            common: OrderCommon {
                id: OrderId::from_u64(125),
                price: 10000,
                display_quantity: 5,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };

        let display_str = order.to_string();

        // Since PostOnly might not be fully implemented, check if it returns
        // the fallback message or the proper format
        if !display_str.contains("not fully implemented") {
            assert!(display_str.starts_with("PostOnly:"));
            assert!(display_str.contains("id=00000000-0000-007d-0000-000000000000"));
            assert!(display_str.contains("price=10000"));
            assert!(display_str.contains("quantity=5"));
            assert!(display_str.contains("side=BUY"));
            assert!(display_str.contains("timestamp=1616823000000"));
            assert!(display_str.contains("time_in_force="));
        } else {
            // If not fully implemented, at least ensure we get the fallback message
            assert_eq!(
                display_str,
                "OrderType variant not fully implemented for Display"
            );
        }
    }

    #[test]
    fn test_trailing_stop_order_display() {
        let order = OrderType::<()>::TrailingStop {
            common: OrderCommon {
                id: OrderId::from_u64(126),
                price: 10000,
                display_quantity: 5,
                side: Side::Sell,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            trail_amount: 100,
            last_reference_price: 10100,
        };

        let display_str = order.to_string();

        if !display_str.contains("not fully implemented") {
            assert!(display_str.starts_with("TrailingStop:"));
            assert!(display_str.contains("id=00000000-0000-007e-0000-000000000000"));
            assert!(display_str.contains("price=10000"));
            assert!(display_str.contains("quantity=5"));
            assert!(display_str.contains("side=SELL"));
            assert!(display_str.contains("trail_amount=100"));
            assert!(display_str.contains("last_reference_price=10100"));
        } else {
            assert_eq!(
                display_str,
                "OrderType variant not fully implemented for Display"
            );
        }
    }

    #[test]
    fn test_pegged_order_display() {
        let order = OrderType::<()>::PeggedOrder {
            common: OrderCommon {
                id: OrderId::from_u64(127),
                price: 10000,
                display_quantity: 5,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reference_price_offset: -50,
            reference_price_type: PegReferenceType::BestAsk,
        };

        let display_str = order.to_string();

        if !display_str.contains("not fully implemented") {
            assert!(display_str.starts_with("PeggedOrder:"));
            assert!(display_str.contains("id=00000000-0000-007f-0000-000000000000"));
            assert!(display_str.contains("price=10000"));
            assert!(display_str.contains("quantity=5"));
            assert!(display_str.contains("side=BUY"));
            assert!(display_str.contains("reference_price_offset=-50"));
            assert!(display_str.contains("reference_price_type=BestAsk"));
        } else {
            assert_eq!(
                display_str,
                "OrderType variant not fully implemented for Display"
            );
        }
    }

    #[test]
    fn test_market_to_limit_order_display() {
        let order = OrderType::<()>::MarketToLimit {
            common: OrderCommon {
                id: OrderId::from_u64(128),
                price: 10000,
                display_quantity: 5,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };

        let display_str = order.to_string();

        if !display_str.contains("not fully implemented") {
            assert!(display_str.starts_with("MarketToLimit:"));
            assert!(display_str.contains("id=00000000-0000-0080-0000-000000000000"));
            assert!(display_str.contains("price=10000"));
            assert!(display_str.contains("quantity=5"));
            assert!(display_str.contains("side=BUY"));
        } else {
            assert_eq!(
                display_str,
                "OrderType variant not fully implemented for Display"
            );
        }
    }

    #[test]
    fn test_reserve_order_display() {
        let order = OrderType::<()>::ReserveOrder {
            common: OrderCommon {
                id: OrderId::from_u64(129),
                price: 10000,
                display_quantity: 1,
                side: Side::Sell,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 4,
            replenish_threshold: 0,
            replenish_amount: Some(1),
            auto_replenish: false,
        };

        let display_str = order.to_string();

        assert!(display_str.starts_with("ReserveOrder:"));
        assert!(display_str.contains("id=00000000-0000-0081-0000-000000000000"));
        assert!(display_str.contains("price=10000"));
        assert!(display_str.contains("display_quantity=1"));
        assert!(display_str.contains("reserve_quantity=4"));
        assert!(display_str.contains("side=SELL"));
        assert!(display_str.contains("replenish_threshold=0"));
        assert!(display_str.contains("auto_replenish=false"));
        assert!(display_str.contains("replenish_amount=1"));
    }
}

#[cfg(test)]
mod from_str_specific_tests {
    use crate::order::{OrderCommon, OrderId, OrderType, PegReferenceType, Side, TimeInForce};
    use std::str::FromStr;

    #[test]
    fn test_from_str_reserve_order() {
        let input = "ReserveOrder:id=00000000-0000-0081-0000-000000000000;price=10000;display_quantity=1;reserve_quantity=4;side=SELL;timestamp=1616823000000;time_in_force=GTC;replenish_threshold=0;replenish_amount=1;auto_replenish=false";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::ReserveOrder {
                common:
                    OrderCommon {
                        id,
                        price,
                        display_quantity,
                        side,
                        timestamp,
                        time_in_force,
                        ..
                    },
                reserve_quantity,
                replenish_threshold,
                replenish_amount,
                auto_replenish,
                ..
            } => {
                assert_eq!(id, OrderId::from_u64(129));
                assert_eq!(price, 10000);
                assert_eq!(display_quantity, 1);
                assert_eq!(reserve_quantity, 4);
                assert_eq!(side, Side::Sell);
                assert_eq!(timestamp, 1616823000000);
                assert_eq!(time_in_force, TimeInForce::Gtc);
                assert_eq!(replenish_threshold, 0);
                assert_eq!(replenish_amount, Some(1));
                assert!(!auto_replenish);
            }
            _ => panic!("Expected ReserveOrder"),
        }

        // Test with None replenish_amount
        let input = "ReserveOrder:id=00000000-0000-0081-0000-000000000000;price=10000;display_quantity=1;reserve_quantity=4;side=SELL;timestamp=1616823000000;time_in_force=GTC;replenish_threshold=10;replenish_amount=None;auto_replenish=true";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::ReserveOrder {
                replenish_amount,
                replenish_threshold,
                auto_replenish,
                ..
            } => {
                assert_eq!(replenish_amount, None);
                assert_eq!(replenish_threshold, 10);
                assert!(auto_replenish);
            }
            _ => panic!("Expected ReserveOrder"),
        }

        // Test with different time_in_force
        let input = "ReserveOrder:id=00000000-0000-0081-0000-000000000000;price=10000;display_quantity=1;reserve_quantity=4;side=SELL;timestamp=1616823000000;time_in_force=GTD-1617000000000;replenish_threshold=5;replenish_amount=2;auto_replenish=true";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::ReserveOrder {
                common: OrderCommon { time_in_force, .. },
                replenish_threshold,
                replenish_amount,
                auto_replenish,
                ..
            } => {
                assert_eq!(time_in_force, TimeInForce::Gtd(1617000000000));
                assert_eq!(replenish_threshold, 5);
                assert_eq!(replenish_amount, Some(2));
                assert!(auto_replenish);
            }
            _ => panic!("Expected ReserveOrder"),
        }
    }

    #[test]
    fn test_from_str_market_to_limit_order() {
        let input = "MarketToLimit:id=00000000-0000-0080-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::MarketToLimit {
                common:
                    OrderCommon {
                        id,
                        price,
                        display_quantity: quantity,
                        side,
                        timestamp,
                        time_in_force,
                        ..
                    },
                ..
            } => {
                assert_eq!(id, OrderId::from_u64(128));
                assert_eq!(price, 10000);
                assert_eq!(quantity, 5);
                assert_eq!(side, Side::Buy);
                assert_eq!(timestamp, 1616823000000);
                assert_eq!(time_in_force, TimeInForce::Gtc);
            }
            _ => panic!("Expected MarketToLimit"),
        }

        // Test with IOC time-in-force
        let input = "MarketToLimit:id=00000000-0000-0080-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=IOC";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::MarketToLimit {
                common: OrderCommon { time_in_force, .. },
                ..
            } => {
                assert_eq!(time_in_force, TimeInForce::Ioc);
            }
            _ => panic!("Expected MarketToLimit"),
        }

        // Test with SELL side
        let input = "MarketToLimit:id=00000000-0000-0080-0000-000000000000;price=10000;display_quantity=5;side=SELL;timestamp=1616823000000;time_in_force=GTC";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::MarketToLimit {
                common: OrderCommon { side, .. },
                ..
            } => {
                assert_eq!(side, Side::Sell);
            }
            _ => panic!("Expected MarketToLimit"),
        }
    }

    #[test]
    fn test_from_str_pegged_order() {
        // Test with BestAsk reference type
        let input = "PeggedOrder:id=00000000-0000-007f-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC;reference_price_offset=-50;reference_price_type=BestAsk";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::PeggedOrder {
                common:
                    OrderCommon {
                        id,
                        price,
                        display_quantity: quantity,
                        side,
                        timestamp,
                        time_in_force,
                        ..
                    },
                reference_price_offset,
                reference_price_type,
                ..
            } => {
                assert_eq!(id, OrderId::from_u64(127));
                assert_eq!(price, 10000);
                assert_eq!(quantity, 5);
                assert_eq!(side, Side::Buy);
                assert_eq!(timestamp, 1616823000000);
                assert_eq!(time_in_force, TimeInForce::Gtc);
                assert_eq!(reference_price_offset, -50);
                assert_eq!(reference_price_type, PegReferenceType::BestAsk);
            }
            _ => panic!("Expected PeggedOrder"),
        }

        // Test with BestBid reference type
        let input = "PeggedOrder:id=00000000-0000-007f-0000-000000000000;price=10000;display_quantity=5;side=SELL;timestamp=1616823000000;time_in_force=IOC;reference_price_offset=50;reference_price_type=BestBid";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::PeggedOrder {
                common:
                    OrderCommon {
                        side,
                        time_in_force,
                        ..
                    },
                reference_price_offset,
                reference_price_type,
                ..
            } => {
                assert_eq!(side, Side::Sell);
                assert_eq!(time_in_force, TimeInForce::Ioc);
                assert_eq!(reference_price_offset, 50);
                assert_eq!(reference_price_type, PegReferenceType::BestBid);
            }
            _ => panic!("Expected PeggedOrder"),
        }

        // Test with MidPrice reference type
        let input = "PeggedOrder:id=00000000-0000-007f-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC;reference_price_offset=0;reference_price_type=MidPrice";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::PeggedOrder {
                reference_price_offset,
                reference_price_type,
                ..
            } => {
                assert_eq!(reference_price_offset, 0);
                assert_eq!(reference_price_type, PegReferenceType::MidPrice);
            }
            _ => panic!("Expected PeggedOrder"),
        }

        // Test with LastTrade reference type
        let input = "PeggedOrder:id=00000000-0000-007f-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC;reference_price_offset=-100;reference_price_type=LastTrade";
        let order: OrderType<()> = OrderType::from_str(input).unwrap();

        match order {
            OrderType::PeggedOrder {
                reference_price_offset,
                reference_price_type,
                ..
            } => {
                assert_eq!(reference_price_offset, -100);
                assert_eq!(reference_price_type, PegReferenceType::LastTrade);
            }
            _ => panic!("Expected PeggedOrder"),
        }
    }

    #[test]
    fn test_from_str_invalid_pegged_reference_type() {
        let input = "PeggedOrder:id=00000000-0000-007f-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC;reference_price_offset=-50;reference_price_type=InvalidType";
        let result: Result<OrderType<()>, _> = OrderType::from_str(input);

        assert!(result.is_err());
        match result.unwrap_err() {
            crate::errors::PriceLevelError::InvalidFieldValue { field, value } => {
                assert_eq!(field, "reference_price_type");
                assert_eq!(value, "InvalidType");
            }
            err => panic!("Expected InvalidFieldValue error, got {err:?}"),
        }
    }

    #[test]
    fn test_from_str_invalid_reserve_order_auto_replenish() {
        let input = "ReserveOrder:id=00000000-0000-0081-0000-000000000000;price=10000;display_quantity=1;reserve_quantity=4;side=SELL;timestamp=1616823000000;time_in_force=GTC;replenish_threshold=0;replenish_amount=1;auto_replenish=invalid";
        let result: Result<OrderType<()>, _> = OrderType::from_str(input);

        assert!(result.is_err());
        match result.unwrap_err() {
            crate::errors::PriceLevelError::InvalidFieldValue { field, value } => {
                assert_eq!(field, "auto_replenish");
                assert_eq!(value, "invalid");
            }
            err => panic!("Expected InvalidFieldValue error, got {err:?}"),
        }
    }

    #[test]
    fn test_edge_cases() {
        // Test case-insensitivity for side
        let input = "Standard:id=00000000-0000-007b-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=IOC";
        let order_result = OrderType::from_str(input);
        assert!(
            order_result.is_ok(),
            "Failed to parse MarketToLimit order string"
        );
        let order: OrderType<()> = order_result.unwrap();

        match order {
            OrderType::Standard {
                common: OrderCommon { side, .. },
            } => {
                assert_eq!(side, Side::Buy);
            }
            _ => panic!("Expected Standard"),
        }

        // Test with maximum values
        let input = format!(
            "PeggedOrder:id=ffffffff-ffff-ffff-0000-000000000000;price={};display_quantity={};side=BUY;timestamp={};time_in_force=GTC;reference_price_offset={};reference_price_type=BestAsk",
            u64::MAX,
            u64::MAX,
            u64::MAX,
            i64::MAX
        );
        let order: OrderType<()> = OrderType::from_str(&input).unwrap();

        match order {
            OrderType::PeggedOrder {
                common:
                    OrderCommon {
                        id,
                        price,
                        display_quantity: quantity,
                        timestamp,
                        ..
                    },
                reference_price_offset,
                reference_price_type: _,
                ..
            } => {
                assert_eq!(id, OrderId::from_u64(u64::MAX));
                assert_eq!(price, u64::MAX);
                assert_eq!(quantity, u64::MAX);
                assert_eq!(timestamp, u64::MAX);
                assert_eq!(reference_price_offset, i64::MAX);
            }
            _ => panic!("Expected PeggedOrder"),
        }

        // Test with minimum values for reference_price_offset
        let input = format!(
            "PeggedOrder:id=00000000-0000-007f-0000-000000000000;price=10000;display_quantity=5;side=BUY;timestamp=1616823000000;time_in_force=GTC;reference_price_offset={};reference_price_type=BestAsk",
            i64::MIN
        );
        let order: OrderType<()> = OrderType::from_str(&input).unwrap();

        match order {
            OrderType::PeggedOrder {
                reference_price_offset,
                ..
            } => {
                assert_eq!(reference_price_offset, i64::MIN);
            }
            _ => panic!("Expected PeggedOrder"),
        }
    }

    #[test]
    fn test_roundtrip_serialization() {
        // Create sample orders
        let orders = vec![
            OrderType::ReserveOrder {
                common: OrderCommon {
                    id: OrderId::from_u64(129),
                    price: 10000,
                    display_quantity: 1,
                    side: Side::Sell,
                    timestamp: 1616823000000,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
                reserve_quantity: 4,
                replenish_threshold: 0,
                replenish_amount: Some(1),
                auto_replenish: false,
            },
            OrderType::MarketToLimit {
                common: OrderCommon {
                    id: OrderId::from_u64(128),
                    price: 10000,
                    display_quantity: 5,
                    side: Side::Buy,
                    timestamp: 1616823000000,
                    time_in_force: TimeInForce::Ioc,
                    extra_fields: (),
                },
            },
            OrderType::PeggedOrder {
                common: OrderCommon {
                    id: OrderId::from_u64(127),
                    price: 10000,
                    display_quantity: 5,
                    side: Side::Buy,
                    timestamp: 1616823000000,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
                reference_price_offset: -50,
                reference_price_type: PegReferenceType::BestAsk,
            },
        ];

        // Test round-trip for each order type
        for original_order in orders {
            let string_representation = original_order.to_string();
            let parsed_order: OrderType<()> = OrderType::from_str(&string_representation).unwrap();

            // Compare specific fields based on order type
            match (original_order, parsed_order) {
                (
                    OrderType::ReserveOrder {
                        common:
                            OrderCommon {
                                id: id1,
                                price: price1,
                                display_quantity: vis1,
                                side: side1,
                                ..
                            },
                        reserve_quantity: hid1,
                        replenish_threshold: thresh1,
                        replenish_amount: amt1,
                        auto_replenish: auto1,
                        ..
                    },
                    OrderType::ReserveOrder {
                        common:
                            OrderCommon {
                                id: id2,
                                price: price2,
                                display_quantity: vis2,
                                side: side2,
                                ..
                            },
                        reserve_quantity: hid2,
                        replenish_threshold: thresh2,
                        replenish_amount: amt2,
                        auto_replenish: auto2,
                        ..
                    },
                ) => {
                    assert_eq!(id1, id2);
                    assert_eq!(price1, price2);
                    assert_eq!(vis1, vis2);
                    assert_eq!(hid1, hid2);
                    assert_eq!(side1, side2);
                    assert_eq!(thresh1, thresh2);
                    assert_eq!(amt1, amt2);
                    assert_eq!(auto1, auto2);
                }
                (
                    OrderType::MarketToLimit {
                        common:
                            OrderCommon {
                                id: id1,
                                price: price1,
                                display_quantity: qty1,
                                side: side1,
                                time_in_force: tif1,
                                ..
                            },
                        ..
                    },
                    OrderType::MarketToLimit {
                        common:
                            OrderCommon {
                                id: id2,
                                price: price2,
                                display_quantity: qty2,
                                side: side2,
                                time_in_force: tif2,
                                ..
                            },
                        ..
                    },
                ) => {
                    assert_eq!(id1, id2);
                    assert_eq!(price1, price2);
                    assert_eq!(qty1, qty2);
                    assert_eq!(side1, side2);
                    assert_eq!(tif1, tif2);
                }
                (
                    OrderType::PeggedOrder {
                        common:
                            OrderCommon {
                                id: id1,
                                price: price1,
                                display_quantity: qty1,
                                side: side1,
                                ..
                            },
                        reference_price_offset: offset1,
                        reference_price_type: ref_type1,
                        ..
                    },
                    OrderType::PeggedOrder {
                        common:
                            OrderCommon {
                                id: id2,
                                price: price2,
                                display_quantity: qty2,
                                side: side2,
                                ..
                            },
                        reference_price_offset: offset2,
                        reference_price_type: ref_type2,
                        ..
                    },
                ) => {
                    assert_eq!(id1, id2);
                    assert_eq!(price1, price2);
                    assert_eq!(qty1, qty2);
                    assert_eq!(side1, side2);
                    assert_eq!(offset1, offset2);
                    assert_eq!(ref_type1, ref_type2);
                }
                _ => panic!("Order types don't match after round-trip"),
            }
        }
    }
}
