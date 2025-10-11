//! Limit order type definitions

use crate::OrderQueue;
use crate::errors::PriceLevelError;
use crate::orders::{OrderId, PegReferenceType, Side, TimeInForce};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

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
