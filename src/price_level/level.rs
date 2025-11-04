//! Core price level implementation

use crate::UuidGenerator;
use crate::errors::PriceLevelError;
use crate::execution::{MatchResult, Transaction};
use crate::order::{Order, OrderId, OrderUpdate};
use crate::price_level::order_queue::OrderQueue;
use crate::price_level::{PriceLevelSnapshot, PriceLevelSnapshotPackage, PriceLevelStatistics};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;

use std::rc::Rc;

/// A lock-free implementation of a price level in a limit order book
#[derive(Debug)]
pub struct PriceLevel {
    /// The price of this level
    price: u64,

    /// Total display quantity at this price level
    display_quantity: u64,

    /// Total reserve quantity at this price level
    reserve_quantity: u64,

    /// Number of orders at this price level
    order_count: usize,

    /// Queue of orders at this price level
    orders: OrderQueue,

    /// Statistics for this price level
    stats: PriceLevelStatistics,
}

impl PriceLevel {
    /// Reconstructs a price level directly from a snapshot.
    pub fn from_snapshot(mut snapshot: PriceLevelSnapshot) -> Result<Self, PriceLevelError> {
        snapshot.refresh_aggregates();

        let order_count = snapshot.orders.len();
        let rc_orders: Vec<Rc<Order<()>>> = snapshot
            .orders
            .into_iter()
            .map(|arc_order| {
                // Convert Arc to Rc by cloning the inner Order
                Rc::new(*arc_order)
            })
            .collect();
        let queue = OrderQueue::from(rc_orders);

        Ok(Self {
            price: snapshot.price,
            display_quantity: snapshot.display_quantity,
            reserve_quantity: snapshot.reserve_quantity,
            order_count,
            orders: queue,
            stats: PriceLevelStatistics::new(),
        })
    }

    /// Reconstructs a price level from a checksum-protected snapshot package.
    pub fn from_snapshot_package(
        package: PriceLevelSnapshotPackage,
    ) -> Result<Self, PriceLevelError> {
        let snapshot = package.into_snapshot()?;
        Self::from_snapshot(snapshot)
    }

    /// Restores a price level from its snapshot JSON representation.
    pub fn from_snapshot_json(data: &str) -> Result<Self, PriceLevelError> {
        let package = PriceLevelSnapshotPackage::from_json(data)?;
        Self::from_snapshot_package(package)
    }
    /// Create a new price level
    pub fn new(price: u64) -> Self {
        Self {
            price,
            display_quantity: 0,
            reserve_quantity: 0,
            order_count: 0,
            orders: OrderQueue::new(),
            stats: PriceLevelStatistics::new(),
        }
    }

    /// Get the price of this level
    pub fn price(&self) -> u64 {
        self.price
    }

    /// Get the display quantity
    pub fn display_quantity(&self) -> u64 {
        self.display_quantity
    }

    /// Get the reserve quantity
    pub fn reserve_quantity(&self) -> u64 {
        self.reserve_quantity
    }

    /// Get the total quantity (visible + hidden)
    pub fn total_quantity(&self) -> u64 {
        self.display_quantity + self.reserve_quantity
    }

    /// Get the number of orders
    pub fn order_count(&self) -> usize {
        self.order_count
    }

    /// Get the statistics for this price level
    pub fn stats(&self) -> &PriceLevelStatistics {
        &self.stats
    }

    /// Add an order to this price level
    pub fn add_order(&mut self, order: Order<()>) -> Rc<Order<()>> {
        // Calculate quantities
        let visible_qty = order.display_quantity();
        let hidden_qty = order.reserve_quantity();

        // Update counters
        self.display_quantity += visible_qty;
        self.reserve_quantity += hidden_qty;
        self.order_count += 1;

        // Update statistics
        self.stats.record_order_added();

        // Add to order queue
        let order_rc = Rc::new(order);
        self.orders.push(order_rc.clone());

        order_rc
    }

    /// Creates an iterator over the orders in the price level.
    pub fn iter_orders(&self) -> Vec<Rc<Order<()>>> {
        self.orders.to_vec()
    }

    /// Matches an incoming order against existing orders at this price level.
    ///
    /// This function attempts to match the incoming order quantity against the orders present in the
    /// `OrderQueue`. It iterates through the queue, matching orders until the incoming quantity is
    /// fully filled or the queue is exhausted.  Transactions are generated for each successful match,
    /// and filled orders are removed from the queue.  The function also updates the visible and hidden
    /// quantity counters and records statistics for each execution.
    ///
    /// # Arguments
    ///
    /// * `incoming_quantity`: The quantity of the incoming order to be matched.
    /// * `taker_order_id`: The ID of the incoming order (the "taker" order).
    /// * `transaction_id_generator`: An atomic counter used to generate unique transaction IDs.
    ///
    /// # Returns
    ///
    /// A `MatchResult` object containing the results of the matching operation, including a list of
    /// generated transactions, the remaining unmatched quantity, a flag indicating whether the
    /// incoming order was completely filled, and a list of IDs of orders that were completely filled
    /// during the matching process.
    pub fn match_order(
        &mut self,
        incoming_quantity: u64,
        taker_order_id: OrderId,
        transaction_id_generator: &UuidGenerator,
    ) -> MatchResult {
        let mut result = MatchResult::new(taker_order_id, incoming_quantity);
        let mut remaining = incoming_quantity;

        while remaining > 0 {
            if let Some(order_rc) = self.orders.pop() {
                let (consumed, updated_order, hidden_reduced, new_remaining) =
                    order_rc.match_against(remaining);

                if consumed > 0 {
                    // Update display quantity counter
                    self.display_quantity -= consumed;

                    // Use UUID generator directly
                    let transaction_id = transaction_id_generator.next();

                    let transaction = Transaction::new(
                        transaction_id,
                        taker_order_id,
                        order_rc.id(),
                        self.price,
                        consumed,
                        order_rc.side().opposite(),
                    );

                    result.add_transaction(transaction);

                    // If the order was completely executed, add it to filled_order_ids
                    if updated_order.is_none() {
                        result.add_filled_order_id(order_rc.id());
                    }
                }

                remaining = new_remaining;

                // Calculate waiting time
                let current_time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let waiting_time = current_time.saturating_sub(order_rc.timestamp());

                // update statistics
                self.stats
                    .record_execution(consumed, order_rc.price(), waiting_time);

                if let Some(updated) = updated_order {
                    if hidden_reduced > 0 {
                        self.reserve_quantity -= hidden_reduced;
                        self.display_quantity += hidden_reduced;
                    }

                    self.orders.push(Rc::new(updated));
                } else {
                    self.order_count -= 1;
                    match &*order_rc {
                        Order::IcebergOrder {
                            reserve_quantity, ..
                        } => {
                            if *reserve_quantity > 0 && hidden_reduced == 0 {
                                self.reserve_quantity -= *reserve_quantity;
                            }
                        }
                        Order::ReserveOrder {
                            reserve_quantity, ..
                        } => {
                            if *reserve_quantity > 0 && hidden_reduced == 0 {
                                self.reserve_quantity -= *reserve_quantity;
                            }
                        }
                        _ => {}
                    }
                }
            } else {
                break;
            }
        }

        result.is_complete = remaining == 0;
        result.remaining_quantity = remaining;
        result
    }

    /// Create a snapshot of the current price level state
    pub fn snapshot(&self) -> PriceLevelSnapshot {
        PriceLevelSnapshot {
            price: self.price,
            display_quantity: self.display_quantity(),
            reserve_quantity: self.reserve_quantity(),
            order_count: self.order_count(),
            orders: self
                .iter_orders()
                .into_iter()
                .map(|rc| Arc::new(*rc))
                .collect(),
        }
    }

    /// Serialize the current price level state into a checksum-protected snapshot package.
    pub fn snapshot_package(&self) -> Result<PriceLevelSnapshotPackage, PriceLevelError> {
        PriceLevelSnapshotPackage::new(self.snapshot())
    }

    /// Serialize the current price level state to JSON, including checksum metadata.
    pub fn snapshot_to_json(&self) -> Result<String, PriceLevelError> {
        self.snapshot_package()?.to_json()
    }

    /// Apply an update to an existing order at this price level
    pub fn update_order(
        &mut self,
        update: OrderUpdate,
    ) -> Result<Option<Rc<Order<()>>>, PriceLevelError> {
        match update {
            OrderUpdate::UpdatePrice {
                order_id,
                new_price,
            } => {
                // If price changes, this order needs to be moved to a different price level
                // So we remove it from this level and return it for re-insertion elsewhere
                if new_price != self.price {
                    let order = self.orders.remove(&order_id);

                    if let Some(ref order_rc) = order {
                        // Update counters
                        let old_visible = order_rc.display_quantity();
                        let old_hidden = order_rc.reserve_quantity();
                        self.display_quantity -= old_visible;
                        self.reserve_quantity -= old_hidden;
                        self.order_count -= 1;

                        // Record removal in statistics
                        self.stats.record_order_removed();
                    }

                    Ok(order)
                } else {
                    // If price is the same, this is a no-op at the price level
                    // (Should be handled at the order book level)
                    Err(PriceLevelError::InvalidOperation {
                        message: "Cannot update price to the same value".to_string(),
                    })
                }
            }

            OrderUpdate::UpdateQuantity {
                order_id,
                new_quantity,
            } => {
                // Find the order
                if let Some(order) = self.orders.find(&order_id) {
                    // Get current quantities
                    let old_visible = order.display_quantity();
                    let old_hidden = order.reserve_quantity();

                    // Remove the old order
                    let old_order = match self.orders.remove(&order_id) {
                        Some(order) => order,
                        None => return Ok(None), // Order not found, remove by other thread
                    };

                    // Create updated order with new quantity
                    let new_order = old_order.with_reduced_quantity(new_quantity);

                    // Calculate the new quantities
                    let new_visible = new_order.display_quantity();
                    let new_hidden = new_order.reserve_quantity();

                    // Update atomic counters
                    if old_visible != new_visible {
                        if new_visible > old_visible {
                            self.display_quantity += new_visible - old_visible;
                        } else {
                            self.display_quantity -= old_visible - new_visible;
                        }
                    }

                    if old_hidden != new_hidden {
                        if new_hidden > old_hidden {
                            self.reserve_quantity += new_hidden - old_hidden;
                        } else {
                            self.reserve_quantity -= old_hidden - new_hidden;
                        }
                    }

                    // Add the updated order back to the queue
                    let new_order_rc = Rc::new(new_order);
                    self.orders.push(new_order_rc.clone());

                    return Ok(Some(new_order_rc));
                }

                Ok(None) // Order not found
            }

            OrderUpdate::UpdatePriceAndQuantity {
                order_id,
                new_price,
                new_quantity,
            } => {
                // If price changes, remove the order and let the order book handle re-insertion
                if new_price != self.price {
                    let order = self.orders.remove(&order_id);

                    if let Some(ref order_arc) = order {
                        // Update atomic counters
                        let visible_qty = order_arc.display_quantity();
                        let hidden_qty = order_arc.reserve_quantity();

                        self.display_quantity -= visible_qty;
                        self.reserve_quantity -= hidden_qty;
                        self.order_count -= 1;

                        // Update statistics
                        self.stats.record_order_removed();
                    }
                    Ok(order)
                } else {
                    // If price is the same, just update the quantity (reuse logic)
                    self.update_order(OrderUpdate::UpdateQuantity {
                        order_id,
                        new_quantity,
                    })
                }
            }

            OrderUpdate::Cancel { order_id } => {
                // Remove the order
                let order = self.orders.remove(&order_id);

                if let Some(ref order_rc) = order {
                    // Update counters
                    let old_visible = order_rc.display_quantity();
                    let old_hidden = order_rc.reserve_quantity();
                    self.display_quantity -= old_visible;
                    self.reserve_quantity -= old_hidden;
                    self.order_count -= 1;

                    // Record removal in statistics
                    self.stats.record_order_removed();
                }

                Ok(order)
            }

            OrderUpdate::Replace {
                order_id,
                price,
                quantity,
                side: _,
            } => {
                // For replacement, check if the price is changing
                if price != self.price {
                    // If price is different, remove the order and let order book handle re-insertion
                    let order = self.orders.remove(&order_id);

                    if let Some(ref order_rc) = order {
                        // Update counters
                        let old_visible = order_rc.display_quantity();
                        let old_hidden = order_rc.reserve_quantity();
                        self.display_quantity -= old_visible;
                        self.reserve_quantity -= old_hidden;
                        self.order_count -= 1;

                        // Record removal in statistics
                        self.stats.record_order_removed();
                    }

                    Ok(order)
                } else {
                    // If price is the same, just update the quantity
                    self.update_order(OrderUpdate::UpdateQuantity {
                        order_id,
                        new_quantity: quantity,
                    })
                }
            }
        }
    }
}

/// Serializable representation of a price level for easier data transfer and storage
#[derive(Debug, Serialize, Deserialize)]
pub struct PriceLevelData {
    /// The price of this level
    pub price: u64,
    /// Total display quantity at this price level
    pub display_quantity: u64,
    /// Total reserve quantity at this price level
    pub reserve_quantity: u64,
    /// Number of orders at this price level
    pub order_count: usize,
    /// Orders at this price level
    pub orders: Vec<Order<()>>,
}

impl From<&PriceLevel> for PriceLevelData {
    fn from(price_level: &PriceLevel) -> Self {
        Self {
            price: price_level.price(),
            display_quantity: price_level.display_quantity(),
            reserve_quantity: price_level.reserve_quantity(),
            order_count: price_level.order_count(),
            orders: price_level
                .iter_orders()
                .into_iter()
                .map(|order_arc| *order_arc)
                .collect(),
        }
    }
}

impl From<&PriceLevelSnapshot> for PriceLevel {
    fn from(snapshot: &PriceLevelSnapshot) -> Self {
        let mut snapshot = snapshot.clone();
        snapshot.refresh_aggregates();

        let rc_orders: Vec<Rc<Order<()>>> = snapshot
            .orders
            .into_iter()
            .map(|arc| Rc::new(*arc))
            .collect();
        let queue = OrderQueue::from(rc_orders);
        let order_count = queue.len();

        Self {
            price: snapshot.price,
            display_quantity: snapshot.display_quantity,
            reserve_quantity: snapshot.reserve_quantity,
            order_count,
            orders: queue,
            stats: PriceLevelStatistics::new(),
        }
    }
}

impl TryFrom<PriceLevelData> for PriceLevel {
    type Error = PriceLevelError;

    fn try_from(data: PriceLevelData) -> Result<Self, Self::Error> {
        let mut price_level = PriceLevel::new(data.price);

        // Add orders to the price level
        for order in data.orders {
            price_level.add_order(order);
        }

        Ok(price_level)
    }
}

// Implement custom serialization for the atomic types
impl Serialize for PriceLevel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Convert to a serializable representation
        let data: PriceLevelData = self.into();
        data.serialize(serializer)
    }
}

impl FromStr for PriceLevel {
    type Err = PriceLevelError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use std::borrow::Cow;

        if !s.starts_with("PriceLevel:") {
            return Err(PriceLevelError::ParseError {
                message: "Invalid format: missing 'PriceLevel:' prefix".to_string(),
            });
        }

        let content = &s["PriceLevel:".len()..];

        let mut parts = std::collections::HashMap::new();
        let remaining_content: Cow<str>;

        if let Some(orders_start) = content.find("orders=[") {
            let orders_end =
                content[orders_start..]
                    .find(']')
                    .ok_or_else(|| PriceLevelError::ParseError {
                        message: "Invalid format: unclosed orders bracket".to_string(),
                    })?
                    + orders_start;

            let orders_str = &content[orders_start + "orders=[".len()..orders_end];
            parts.insert("orders", orders_str);

            let before_orders = &content[..orders_start];
            let after_orders = &content[orders_end + 1..];
            remaining_content = Cow::Owned([before_orders, after_orders].join(""));
        } else {
            remaining_content = Cow::Borrowed(content);
        }

        for part in remaining_content.split(';').filter(|s| !s.is_empty()) {
            let mut iter = part.splitn(2, '=');
            if let (Some(key), Some(value)) = (iter.next(), iter.next()) {
                parts.insert(key, value);
            }
        }

        let price = parts
            .get("price")
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| PriceLevelError::ParseError {
                message: "Missing or invalid price".to_string(),
            })?;

        let mut price_level = PriceLevel::new(price);

        if let Some(orders_part) = parts.get("orders")
            && !orders_part.is_empty()
        {
            let mut bracket_level = 0;
            let mut last_split = 0;

            for (i, c) in orders_part.char_indices() {
                match c {
                    '(' | '[' => bracket_level += 1,
                    ')' | ']' => bracket_level -= 1,
                    ',' if bracket_level == 0 => {
                        let order_str = &orders_part[last_split..i];
                        let order = Order::<()>::from_str(order_str).map_err(|e| {
                            PriceLevelError::ParseError {
                                message: format!("Order parse error: {e}"),
                            }
                        })?;
                        price_level.add_order(order);
                        last_split = i + 1;
                    }
                    _ => {}
                }
            }

            let order_str = &orders_part[last_split..];
            if !order_str.is_empty() {
                let order =
                    Order::<()>::from_str(order_str).map_err(|e| PriceLevelError::ParseError {
                        message: format!("Order parse error: {e}"),
                    })?;
                price_level.add_order(order);
            }
        }

        Ok(price_level)
    }
}

impl<'de> Deserialize<'de> for PriceLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Deserialize into the data representation
        let data = PriceLevelData::deserialize(deserializer)?;

        // Convert to PriceLevel
        PriceLevel::try_from(data).map_err(serde::de::Error::custom)
    }
}

impl PartialEq for PriceLevel {
    fn eq(&self, other: &Self) -> bool {
        self.price == other.price
    }
}

impl Eq for PriceLevel {}

impl PartialOrd for PriceLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PriceLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.price.cmp(&other.price)
    }
}

impl Display for PriceLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let orders_str: Vec<String> = self.iter_orders().iter().map(|o| o.to_string()).collect();
        write!(
            f,
            "PriceLevel:price={};display_quantity={};reserve_quantity={};order_count={};orders=[{}]",
            self.price(),
            self.display_quantity(),
            self.reserve_quantity(),
            self.order_count(),
            orders_str.join(",")
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::errors::PriceLevelError;
    use crate::order::{
        Order, OrderCommon, OrderId, OrderUpdate, PegReferenceType, Side, TimeInForce,
    };
    use crate::price_level::level::{PriceLevel, PriceLevelData};
    use crate::price_level::snapshot::SNAPSHOT_FORMAT_VERSION;
    use crate::{DEFAULT_RESERVE_REPLENISH_AMOUNT, UuidGenerator};
    use std::str::FromStr;
    use tracing::error;
    use uuid::Uuid;

    // Shared timestamp counter for all order creation functions to ensure proper ordering
    static TIMESTAMP_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1616823000000);

    // Helper functions to create different order types for testing
    pub fn create_standard_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        let order_id = OrderId::from_u64(id);
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::Standard {
            common: OrderCommon {
                id: order_id,
                price,
                display_quantity: quantity,
                side: Side::Buy,
                timestamp,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        }
    }

    #[test]
    fn test_price_level_snapshot_roundtrip() {
        let mut price_level = PriceLevel::new(10000);
        price_level.add_order(create_standard_order(1, 10000, 100));
        price_level.add_order(create_iceberg_order(2, 10000, 50, 200));

        let package = price_level
            .snapshot_package()
            .expect("Failed to create snapshot package");

        assert_eq!(package.version, SNAPSHOT_FORMAT_VERSION);
        package.validate().expect("Snapshot validation failed");

        let json = package
            .to_json()
            .expect("Failed to serialize snapshot package");
        let restored = PriceLevel::from_snapshot_json(&json)
            .expect("Failed to restore price level from snapshot JSON");

        assert_eq!(restored.price(), price_level.price());
        assert_eq!(restored.display_quantity(), price_level.display_quantity());
        assert_eq!(restored.reserve_quantity(), price_level.reserve_quantity());
        assert_eq!(restored.order_count(), price_level.order_count());

        let original_ids: Vec<OrderId> = price_level
            .iter_orders()
            .into_iter()
            .map(|order| order.id())
            .collect();
        let restored_ids: Vec<OrderId> = restored
            .iter_orders()
            .into_iter()
            .map(|order| order.id())
            .collect();
        assert_eq!(restored_ids, original_ids);
    }

    #[test]
    fn test_price_level_snapshot_checksum_failure() {
        let mut price_level = PriceLevel::new(20000);
        price_level.add_order(create_standard_order(1, 20000, 100));

        let mut package = price_level
            .snapshot_package()
            .expect("Failed to create snapshot package");

        package.validate().expect("Snapshot validation should pass");

        // Corrupt the checksum and ensure validation fails
        package.checksum = "deadbeef".to_string();
        let err = PriceLevel::from_snapshot_package(package)
            .expect_err("Restoration should fail due to checksum mismatch");

        assert!(matches!(err, PriceLevelError::ChecksumMismatch { .. }));
    }

    #[test]
    fn test_price_level_from_snapshot_preserves_order_positions() {
        let mut price_level = PriceLevel::new(15000);
        price_level.add_order(create_standard_order(1, 15000, 100));
        price_level.add_order(create_iceberg_order(2, 15000, 40, 120));
        price_level.add_order(create_post_only_order(3, 15000, 60));
        price_level.add_order(create_reserve_order(4, 15000, 30, 90, 15, true, Some(20)));

        let snapshot = price_level.snapshot();
        let restored = PriceLevel::from(&snapshot);

        let original_orders = price_level.iter_orders();
        let restored_orders = restored.iter_orders();

        assert_eq!(restored_orders.len(), original_orders.len());
        assert_eq!(restored.order_count(), price_level.order_count());
        assert_eq!(restored.display_quantity(), price_level.display_quantity());
        assert_eq!(restored.reserve_quantity(), price_level.reserve_quantity());

        for (index, (expected, actual)) in original_orders
            .iter()
            .zip(restored_orders.iter())
            .enumerate()
        {
            assert_eq!(
                actual.id(),
                expected.id(),
                "Order mismatch at position {index}"
            );
            assert_eq!(actual.timestamp(), expected.timestamp());
        }
    }

    #[test]
    fn test_price_level_from_snapshot_package_preserves_order_positions() {
        let mut price_level = PriceLevel::new(17500);
        price_level.add_order(create_standard_order(10, 17500, 80));
        price_level.add_order(create_trailing_stop_order(11, 17500, 50));
        price_level.add_order(create_pegged_order(12, 17500, 40));
        price_level.add_order(create_market_to_limit_order(13, 17500, 70));

        let package = price_level
            .snapshot_package()
            .expect("Failed to create snapshot package");
        let restored = PriceLevel::from_snapshot_package(package)
            .expect("Failed to restore price level from snapshot package");

        let original_orders = price_level.iter_orders();
        let restored_orders = restored.iter_orders();

        assert_eq!(restored_orders.len(), original_orders.len());
        assert_eq!(restored.order_count(), price_level.order_count());

        for (index, (expected, actual)) in original_orders
            .iter()
            .zip(restored_orders.iter())
            .enumerate()
        {
            assert_eq!(
                actual.id(),
                expected.id(),
                "Order mismatch at position {index}"
            );
            assert_eq!(actual.timestamp(), expected.timestamp());
        }
    }

    fn create_iceberg_order(id: u64, price: u64, visible: u64, hidden: u64) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::IcebergOrder {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: visible,
                side: Side::Sell,
                timestamp,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: hidden,
        }
    }

    fn create_post_only_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::PostOnly {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: quantity,
                side: Side::Buy,
                timestamp,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        }
    }

    fn create_trailing_stop_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::TrailingStop {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: quantity,
                side: Side::Sell,
                timestamp,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            trail_amount: 100,
            last_reference_price: price + 100,
        }
    }

    fn create_pegged_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::PeggedOrder {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: quantity,
                side: Side::Buy,
                timestamp,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reference_price_offset: -50,
            reference_price_type: PegReferenceType::BestAsk,
        }
    }

    fn create_market_to_limit_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::MarketToLimit {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: quantity,
                side: Side::Buy,
                timestamp,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        }
    }

    fn create_reserve_order(
        id: u64,
        price: u64,
        visible: u64,
        hidden: u64,
        threshold: u64,
        auto_replenish: bool,
        replenish_amount: Option<u64>,
    ) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::ReserveOrder {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: visible,
                side: Side::Sell,
                timestamp,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: hidden,
            replenish_threshold: threshold,
            replenish_amount,
            auto_replenish,
        }
    }

    fn create_fill_or_kill_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: quantity,
                side: Side::Buy,
                timestamp,
                time_in_force: TimeInForce::Fok,
                extra_fields: (),
            },
        }
    }

    fn create_immediate_or_cancel_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: quantity,
                side: Side::Buy,
                timestamp,
                time_in_force: TimeInForce::Ioc,
                extra_fields: (),
            },
        }
    }

    fn create_good_till_date_order(id: u64, price: u64, quantity: u64, expiry: u64) -> Order<()> {
        let timestamp = TIMESTAMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Order::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: quantity,
                side: Side::Buy,
                timestamp,
                time_in_force: TimeInForce::Gtd(expiry),
                extra_fields: (),
            },
        }
    }

    #[test]
    fn test_price_level_creation() {
        let price_level = PriceLevel::new(10000);

        assert_eq!(price_level.price(), 10000);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
        assert_eq!(price_level.total_quantity(), 0);

        // Test the statistics are properly initialized
        let stats = price_level.stats();
        assert_eq!(stats.orders_added(), 0);
        assert_eq!(stats.orders_removed(), 0);
        assert_eq!(stats.orders_executed(), 0);
    }

    #[test]
    fn test_add_standard_order() {
        let mut price_level = PriceLevel::new(10000);
        let order = create_standard_order(1, 10000, 100);

        let order_arc = price_level.add_order(order);

        assert_eq!(price_level.display_quantity(), 100);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 1);
        assert_eq!(price_level.total_quantity(), 100);

        // Verify the returned Arc contains the expected order
        assert_eq!(order_arc.id(), OrderId::from_u64(1));
        assert_eq!(order_arc.price(), 10000);
        assert_eq!(order_arc.display_quantity(), 100);

        // Verify stats
        assert_eq!(price_level.stats().orders_added(), 1);
    }

    #[test]
    fn test_add_iceberg_order() {
        let mut price_level = PriceLevel::new(10000);
        let order = create_iceberg_order(2, 10000, 50, 200);

        price_level.add_order(order);

        assert_eq!(price_level.display_quantity(), 50);
        assert_eq!(price_level.reserve_quantity(), 200);
        assert_eq!(price_level.order_count(), 1);
        assert_eq!(price_level.total_quantity(), 250);
    }

    #[test]
    fn test_add_multiple_orders() {
        let mut price_level = PriceLevel::new(10000);

        // Add different order types
        price_level.add_order(create_standard_order(1, 10000, 100));
        price_level.add_order(create_iceberg_order(2, 10000, 50, 200));
        price_level.add_order(create_post_only_order(3, 10000, 75));
        price_level.add_order(create_reserve_order(4, 10000, 25, 100, 100, true, None));

        assert_eq!(price_level.display_quantity(), 250); // 100 + 50 + 75 + 25
        assert_eq!(price_level.reserve_quantity(), 300); // 0 + 200 + 0 + 100
        assert_eq!(price_level.order_count(), 4);
        assert_eq!(price_level.total_quantity(), 550);

        // Verify stats
        assert_eq!(price_level.stats().orders_added(), 4);
    }

    #[test]
    fn test_update_order_cancel() {
        let mut price_level = PriceLevel::new(10000);

        price_level.add_order(create_standard_order(1, 10000, 100));
        price_level.add_order(create_iceberg_order(2, 10000, 50, 200));

        // Cancel the standard order using OrderUpdate
        let result = price_level.update_order(OrderUpdate::Cancel {
            order_id: OrderId::from_u64(1),
        });

        assert!(result.is_ok());
        let removed = result.unwrap();
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id(), OrderId::from_u64(1));
        assert_eq!(price_level.display_quantity(), 50);
        assert_eq!(price_level.reserve_quantity(), 200);
        assert_eq!(price_level.order_count(), 1);

        // Cancel the iceberg order
        let result = price_level.update_order(OrderUpdate::Cancel {
            order_id: OrderId::from_u64(2),
        });

        assert!(result.is_ok());
        let removed = result.unwrap();
        assert!(removed.is_some());
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);

        // Try to cancel a non-existent order
        let result = price_level.update_order(OrderUpdate::Cancel {
            order_id: OrderId::from_u64(3),
        });

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());

        // Verify stats
        assert_eq!(price_level.stats().orders_added(), 2);
        assert_eq!(price_level.stats().orders_removed(), 2);
    }

    #[test]
    fn test_iter_orders() {
        let mut price_level = PriceLevel::new(10000);

        price_level.add_order(create_standard_order(1, 10000, 100));
        price_level.add_order(create_iceberg_order(2, 10000, 50, 200));

        let orders = price_level.iter_orders();

        assert_eq!(orders.len(), 2);
        assert_eq!(orders[0].id(), OrderId::from_u64(1));
        assert_eq!(orders[1].id(), OrderId::from_u64(2));

        // Verify the orders are still in the queue after iteration
        assert_eq!(price_level.order_count(), 2);
    }

    #[test]
    fn test_match_standard_order_full() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_standard_order(1, 10000, 100));

        // Match the entire order
        let taker_id = OrderId::from_u64(999); // market order ID
        let match_result = price_level.match_order(100, taker_id, &transaction_id_generator);

        assert_eq!(match_result.order_id, taker_id);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);

        assert_eq!(match_result.transactions.len(), 1);
        let transaction = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 100);
        assert_eq!(transaction.taker_side, Side::Sell); // Taker is a market order, so it's a sell side opposite of maker

        assert_eq!(match_result.filled_order_ids.len(), 1);
        assert_eq!(match_result.filled_order_ids[0], OrderId::from_u64(1));

        // Verify stats
        assert_eq!(price_level.stats().orders_executed(), 1);
        assert_eq!(price_level.stats().quantity_executed(), 100);
        assert_eq!(price_level.stats().value_executed(), 1000000); // 100 * 10000
    }

    #[test]
    fn test_match_standard_order_partial() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_standard_order(1, 10000, 100));

        // Match part of the order
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(60, taker_id, &transaction_id_generator);

        // Verificar el resultado de matching
        assert_eq!(match_result.order_id, taker_id);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 40);
        assert_eq!(price_level.order_count(), 1);

        // Verificar las transacciones generadas
        assert_eq!(match_result.transactions.len(), 1);
        let transaction = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 60);
        assert_eq!(transaction.taker_side, Side::Sell);

        // Verificar que no hay órdenes completadas
        assert_eq!(match_result.filled_order_ids.len(), 0);

        // Verify stats
        assert_eq!(price_level.stats().orders_executed(), 1);
        assert_eq!(price_level.stats().quantity_executed(), 60);
    }

    #[test]
    fn test_match_standard_order_excess() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_standard_order(1, 10000, 100));

        // Match with quantity exceeding available
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(150, taker_id, &transaction_id_generator);

        assert_eq!(match_result.order_id, taker_id);
        assert_eq!(match_result.remaining_quantity, 50); // 150 - 100 = 50 remaining
        assert!(!match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);

        assert_eq!(match_result.transactions.len(), 1);
        let transaction = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 100);

        assert_eq!(match_result.filled_order_ids.len(), 1);
        assert_eq!(match_result.filled_order_ids[0], OrderId::from_u64(1));
    }

    // ------------------------------------------- ICEBERG ORDERS -------------------------------------------

    #[test]
    /// This test verifies the matching behavior of iceberg orders within a `PriceLevel`.
    /// It focuses on how the visible and hidden quantities are updated during matching,
    /// and how transactions are generated.  It also checks the state of the `PriceLevel`
    /// after each match, including visible/hidden quantities and the number of orders.
    fn test_match_iceberg_order() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Add a new iceberg order with a visible quantity of 50 and a hidden quantity of 100.
        price_level.add_order(create_iceberg_order(1, 10000, 50, 100));

        // Match the visible portion of the iceberg order.
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        // Assertions to validate the match result.
        assert_eq!(match_result.order_id, taker_id);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 50);
        assert_eq!(price_level.reserve_quantity(), 50); // Hidden quantity reduced
        assert_eq!(price_level.order_count(), 1);
        assert_eq!(match_result.transactions.len(), 1);

        // Assertions about the generated transaction
        let transaction = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 50);
        assert_eq!(transaction.taker_side, Side::Buy);
        assert_eq!(match_result.filled_order_ids.len(), 0);

        // Match another 50 units, which should deplete the visible portion and reveal more.
        let taker_id = OrderId::from_u64(1000);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 50); // Visible quantity replenished
        assert_eq!(price_level.reserve_quantity(), 0); // Hidden quantity reduced
        assert_eq!(price_level.order_count(), 1);
        let transaction = &match_result.transactions.as_vec()[0];

        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 50);
        assert_eq!(transaction.taker_side, Side::Buy);
        assert_eq!(match_result.filled_order_ids.len(), 0);

        // Match the remaining 50 units (50 visible + 0 hidden).
        let taker_id = OrderId::from_u64(1001);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
        assert_eq!(match_result.filled_order_ids.len(), 1);
        assert_eq!(match_result.filled_order_ids[0], OrderId::from_u64(1));
    }

    #[test]
    fn test_match_iceberg_order_overlapping() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Add a new iceberg order with a visible quantity of 50 and a hidden quantity of 100.
        price_level.add_order(create_iceberg_order(1, 10000, 100, 100));

        // Match the visible portion of the iceberg order.
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        // Assertions to validate the match result.
        assert_eq!(match_result.order_id, taker_id);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 50);
        assert_eq!(price_level.reserve_quantity(), 100); // Hidden quantity reduced
        assert_eq!(price_level.order_count(), 1);
        assert_eq!(match_result.transactions.len(), 1);

        // Assertions about the generated transaction
        let transaction = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 50);
        assert_eq!(transaction.taker_side, Side::Buy);
        assert_eq!(match_result.filled_order_ids.len(), 0);

        // Match another 50 units, which should deplete the visible portion and reveal more.
        let taker_id = OrderId::from_u64(1000);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 50); // Visible quantity replenished
        assert_eq!(price_level.reserve_quantity(), 50); // Hidden quantity reduced
        assert_eq!(price_level.order_count(), 1);
        let transaction = &match_result.transactions.as_vec()[0];

        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 50);
        assert_eq!(transaction.taker_side, Side::Buy);
        assert_eq!(match_result.filled_order_ids.len(), 0);

        // Match the remaining 50 units (50 visible + 0 hidden).
        let taker_id = OrderId::from_u64(1001);

        // This should match the remaining visible quantity and deplete the hidden quantity.
        let match_result = price_level.match_order(150, taker_id, &transaction_id_generator);
        assert_eq!(match_result.remaining_quantity, 50);
        assert!(!match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
        assert_eq!(match_result.filled_order_ids.len(), 1);
        assert_eq!(match_result.filled_order_ids[0], OrderId::from_u64(1));
    }

    #[test]
    fn test_match_iceberg_order_partial_visible() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_iceberg_order(1, 10000, 50, 150));

        // Match part of the visible portion
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(30, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 20);
        assert_eq!(price_level.reserve_quantity(), 150); // Hidden unchanged
        assert_eq!(price_level.order_count(), 1);
    }

    // ------------------------------------------- RESERVE ORDERS -------------------------------------------

    #[test]
    /// Tests the behavior of a Reserve Order with auto-replenish disabled.
    /// When the visible quantity is consumed completely, the order should be removed
    /// from the price level even if there is remaining hidden quantity.
    fn test_match_reserve_order_no_auto_replenish() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with auto-replenish disabled
        price_level.add_order(create_reserve_order(1, 10000, 50, 150, 20, false, None));

        // Match the entire visible portion
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        // The order should be removed since the visible quantity reached 0 and auto_replenish is false
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
    }

    #[test]
    /// Tests the behavior of a Reserve Order with auto-replenish enabled.
    /// When the visible quantity is fully consumed, the order should automatically
    /// replenish from the hidden quantity.
    fn test_match_reserve_order_with_auto_replenish() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with auto-replenish enabled
        price_level.add_order(create_reserve_order(1, 10000, 50, 150, 20, true, None));

        // Match the entire visible portion
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        // The order should be replenished with the default amount
        assert_eq!(
            price_level.display_quantity(),
            DEFAULT_RESERVE_REPLENISH_AMOUNT
        );
        assert_eq!(
            price_level.reserve_quantity(),
            150 - DEFAULT_RESERVE_REPLENISH_AMOUNT
        );
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    /// Tests partial matching of a Reserve Order with auto-replenish disabled.
    /// Verifies that the visible quantity decreases correctly and there is no automatic
    /// replenishment even when falling below the threshold.
    fn test_match_reserve_order_partial_no_replenish() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with auto-replenish disabled
        price_level.add_order(create_reserve_order(1, 10000, 50, 150, 20, false, None));

        // Match partially, but still above threshold
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(25, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 25); // 50 - 25 = 25
        assert_eq!(price_level.reserve_quantity(), 150); // No change to hidden quantity

        // Match more to go below threshold
        let taker_id = OrderId::from_u64(1000);
        let match_result = price_level.match_order(10, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        // No automatic replenishment because auto_replenish is false
        assert_eq!(price_level.display_quantity(), 15); // 25 - 10 = 15, no replenishment
        assert_eq!(price_level.reserve_quantity(), 150); // No change to hidden quantity
    }

    #[test]
    /// Tests a Reserve Order with a custom replenishment amount.
    /// When the visible quantity is fully consumed, the order should replenish
    /// using the specified custom amount rather than the default.
    fn test_match_reserve_order_with_custom_replenish_amount() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with auto-replenish enabled and a custom replenishment amount
        let custom_amount = 50;
        price_level.add_order(create_reserve_order(
            1,
            10000,
            50,
            150,
            20,
            true,
            Some(custom_amount),
        ));

        // Match the entire visible portion
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        // The order should be replenished with the custom amount
        assert_eq!(price_level.display_quantity(), custom_amount);
        assert_eq!(price_level.reserve_quantity(), 150 - custom_amount);
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    /// Tests a Reserve Order with threshold 0 and auto-replenish enabled.
    /// A threshold of 0 is treated as 1, but no replenishment should occur
    /// when visible quantity equals the threshold.
    fn test_match_reserve_order_with_zero_threshold() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with threshold 0 and auto-replenish enabled
        price_level.add_order(create_reserve_order(1, 10000, 50, 150, 0, true, None));

        // Match partially
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(49, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        // 1 visible unit will remain, which equals the safe threshold (1), so no replenishment occurs
        assert_eq!(price_level.display_quantity(), 1);
        assert_eq!(price_level.reserve_quantity(), 150);
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    /// Tests a Reserve Order with threshold 0 and auto-replenish disabled.
    /// The order should be removed from the book when visible quantity reaches 0.
    fn test_match_reserve_order_threshold_zero() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with threshold 0 and auto-replenish disabled
        price_level.add_order(create_reserve_order(1, 10000, 50, 150, 0, false, None));

        // Match the entire visible portion
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        // The order should be removed from the price level
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
    }

    #[test]
    /// Tests a Reserve Order with threshold 1 and auto-replenish disabled.
    /// The order should be removed from the book when visible quantity reaches 0.
    fn test_match_reserve_order_threshold_one() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with threshold 1 and auto-replenish disabled
        price_level.add_order(create_reserve_order(1, 10000, 50, 150, 1, false, None));

        // Match the entire visible portion
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        // The order should be removed from the price level
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
    }

    #[test]
    /// Tests a Reserve Order with a specific threshold and auto-replenish disabled.
    /// Verifies behavior when matching above and below the threshold.
    fn test_match_reserve_order_with_threshold() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with threshold 20 and auto-replenish disabled
        price_level.add_order(create_reserve_order(1, 10000, 50, 150, 20, false, None));

        // Match part of the visible portion, but still above threshold
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(25, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 25); // 50 - 25 = 25
        assert_eq!(price_level.reserve_quantity(), 150); // No replenishment yet

        // Match more to go below threshold
        let taker_id = OrderId::from_u64(1000);
        let match_result = price_level.match_order(10, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        // No automatic replenishment because auto_replenish is false
        assert_eq!(price_level.display_quantity(), 15); // 25 - 10 = 15
        assert_eq!(price_level.reserve_quantity(), 150); // No change in hidden quantity
    }

    #[test]
    /// Tests a comprehensive scenario with a Reserve Order including:
    /// 1. Matching above the threshold
    /// 2. Matching below the threshold with automatic replenishment
    /// 3. Matching with an amount larger than available
    ///    This test verifies correct transaction generation and order state throughout.
    fn test_match_reserve_order_overlapping() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Create a reserve order with threshold 20, auto-replenish enabled
        // and default replenish amount (80)
        price_level.add_order(create_reserve_order(1, 10000, 100, 100, 20, true, None));

        // Match 80 units, which is above the replenish threshold
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(80, taker_id, &transaction_id_generator);

        // Validate the match result
        assert_eq!(match_result.order_id, taker_id);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 20); // 100 - 80 = 20
        assert_eq!(price_level.reserve_quantity(), 100); // Hidden quantity unchanged (still above threshold)
        assert_eq!(price_level.order_count(), 1);
        assert_eq!(match_result.transactions.len(), 1);

        // Validate the transaction details
        let transaction = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 80);
        assert_eq!(transaction.taker_side, Side::Buy);
        assert_eq!(match_result.filled_order_ids.len(), 0);

        // Match 10 more units, which will take us below the replenish threshold
        let taker_id = OrderId::from_u64(1000);
        let match_result = price_level.match_order(10, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 90); // 20 - 10 = 10, then replenished to 90 (10 + 80)
        assert_eq!(price_level.reserve_quantity(), 20); // 100 - 80 (replenish amount) = 20
        assert_eq!(price_level.order_count(), 1);

        let transaction = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction.taker_order_id, taker_id);
        assert_eq!(transaction.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction.price, 10000);
        assert_eq!(transaction.quantity, 10);
        assert_eq!(transaction.taker_side, Side::Buy);
        assert_eq!(match_result.filled_order_ids.len(), 0);

        // Match with a larger amount than what's available
        let taker_id = OrderId::from_u64(1001);
        let match_result = price_level.match_order(150, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 40); // 150 - 90 - 20 = 40
        assert!(!match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
        assert_eq!(match_result.filled_order_ids.len(), 1);
        assert_eq!(match_result.filled_order_ids[0], OrderId::from_u64(1));

        // Verify the correct number and sizes of transactions
        assert_eq!(match_result.transactions.len(), 2); // One for visible, one for hidden

        let transaction1 = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction1.taker_order_id, taker_id);
        assert_eq!(transaction1.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction1.price, 10000);
        assert_eq!(transaction1.quantity, 90); // First consumes all visible
        assert_eq!(transaction1.taker_side, Side::Buy);

        let transaction2 = &match_result.transactions.as_vec()[1];
        assert_eq!(transaction2.taker_order_id, taker_id);
        assert_eq!(transaction2.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction2.price, 10000);
        assert_eq!(transaction2.quantity, 20); // Then consumes all hidden
        assert_eq!(transaction2.taker_side, Side::Buy);
    }

    // ------------------------------------------- POST-ONLY, TRAILING STOP, PEGGED, MARKET TO LIMIT, FOK, IOC, GTD ORDERS -------------------------------------------

    #[test]
    fn test_match_post_only_order() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_post_only_order(1, 10000, 100));

        // Post-only orders behave like standard orders for matching
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(60, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 40);
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    fn test_match_trailing_stop_order() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_trailing_stop_order(1, 10000, 100));

        // Trailing stop orders behave like standard orders for matching
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(100, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
    }

    #[test]
    fn test_match_pegged_order() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_pegged_order(1, 10000, 100));

        // Pegged orders behave like standard orders for matching
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 50);
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    fn test_match_market_to_limit_order() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_market_to_limit_order(1, 10000, 100));

        // Market-to-limit orders behave like standard orders for matching
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(100, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
    }

    #[test]
    fn test_match_fill_or_kill_order() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_fill_or_kill_order(1, 10000, 100));

        // For the price level, FOK behaves like standard orders
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(100, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
    }

    #[test]
    fn test_match_immediate_or_cancel_order() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_immediate_or_cancel_order(1, 10000, 100));

        // For the price level, IOC behaves like standard orders
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(50, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 50);
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    fn test_match_good_till_date_order() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_good_till_date_order(1, 10000, 100, 1617000000000));

        // GTD orders behave like standard orders for matching
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(100, taker_id, &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
    }

    #[test]
    fn test_match_multiple_orders() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        price_level.add_order(create_standard_order(1, 10000, 50));
        price_level.add_order(create_standard_order(2, 10000, 75));
        price_level.add_order(create_standard_order(3, 10000, 25));

        // Match first two orders completely and third partially
        let taker_id = OrderId::from_u64(999);
        let match_result = price_level.match_order(140, taker_id, &transaction_id_generator);

        // Verificar el resultado de matching
        assert_eq!(match_result.order_id, taker_id);
        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 10); // 25 - (140 - 50 - 75) = 10
        assert_eq!(price_level.order_count(), 1);

        assert_eq!(match_result.transactions.len(), 3);

        let transaction1 = &match_result.transactions.as_vec()[0];
        assert_eq!(transaction1.taker_order_id, taker_id);
        assert_eq!(transaction1.maker_order_id, OrderId::from_u64(1));
        assert_eq!(transaction1.quantity, 50);

        let transaction2 = &match_result.transactions.as_vec()[1];
        assert_eq!(transaction2.taker_order_id, taker_id);
        assert_eq!(transaction2.maker_order_id, OrderId::from_u64(2));
        assert_eq!(transaction2.quantity, 75);

        let transaction3 = &match_result.transactions.as_vec()[2];
        assert_eq!(transaction3.taker_order_id, taker_id);
        assert_eq!(transaction3.maker_order_id, OrderId::from_u64(3));
        assert_eq!(transaction3.quantity, 15);

        assert_eq!(match_result.filled_order_ids.len(), 2);
        assert!(
            match_result
                .filled_order_ids
                .contains(&OrderId::from_u64(1))
        );
        assert!(
            match_result
                .filled_order_ids
                .contains(&OrderId::from_u64(2))
        );

        let orders = price_level.iter_orders();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].id(), OrderId::from_u64(3));
        assert_eq!(orders[0].display_quantity(), 10);
        assert_eq!(orders[0].reserve_quantity(), 0);
    }

    #[test]
    fn test_snapshot() {
        let mut price_level = PriceLevel::new(10000);

        // Add some orders
        price_level.add_order(create_standard_order(1, 10000, 100));
        price_level.add_order(create_standard_order(2, 10000, 50));

        // Create a snapshot
        let snapshot = price_level.snapshot();

        // Verify snapshot data
        assert_eq!(snapshot.price, 10000);
        assert_eq!(snapshot.display_quantity, 150); // 100 + 50
        assert_eq!(snapshot.reserve_quantity, 0);
        assert_eq!(snapshot.order_count, 2);
        assert_eq!(snapshot.orders.len(), 2);

        // Verify that orders in the snapshot match those in the price level
        let orders_from_level = price_level.iter_orders();
        assert_eq!(snapshot.orders.len(), orders_from_level.len());

        // Check that all orders from the price level are in the snapshot
        for order in orders_from_level {
            let found = snapshot.orders.iter().any(|o| o.id() == order.id());
            assert!(found, "Order with ID {} not found in snapshot", order.id());
        }
    }

    #[test]
    fn test_update_order_update_price() {
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        let order = create_standard_order(1, 10000, 100);
        price_level.add_order(order);

        // Update the price to a different value
        let update = OrderUpdate::UpdatePrice {
            order_id: OrderId::from_u64(1),
            new_price: 11000,
        };

        let result = price_level.update_order(update);

        // The order should be removed from this price level (to be inserted in another price level)
        assert!(result.is_ok());
        let removed_order = result.unwrap();
        assert!(removed_order.is_some());
        assert_eq!(removed_order.unwrap().id(), OrderId::from_u64(1));

        // The price level should now be empty
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);

        // Test updating price to same value (should return error)
        let order = create_standard_order(2, 10000, 100);
        price_level.add_order(order);

        let same_price_update = OrderUpdate::UpdatePrice {
            order_id: OrderId::from_u64(2),
            new_price: 10000,
        };

        let result = price_level.update_order(same_price_update);
        assert!(result.is_err());
        match result {
            Err(PriceLevelError::InvalidOperation { .. }) => (),
            _ => panic!("Expected InvalidOperation error"),
        }
    }

    #[test]
    fn test_update_order_update_quantity() {
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        let order = create_standard_order(1, 10000, 100);
        price_level.add_order(order);

        // Update to increase quantity
        let update = OrderUpdate::UpdateQuantity {
            order_id: OrderId::from_u64(1),
            new_quantity: 150,
        };

        let result = price_level.update_order(update);

        // The order should be updated with the new quantity
        assert!(result.is_ok());
        let updated_order = result.unwrap();
        assert!(updated_order.is_some());
        assert_eq!(updated_order.unwrap().display_quantity(), 150);

        // The price level should reflect the new quantity
        assert_eq!(price_level.display_quantity(), 150);
        assert_eq!(price_level.order_count(), 1);

        // Update to decrease quantity
        let update = OrderUpdate::UpdateQuantity {
            order_id: OrderId::from_u64(1),
            new_quantity: 50,
        };

        let result = price_level.update_order(update);

        // The order should be updated with the new quantity
        assert!(result.is_ok());
        let updated_order = result.unwrap();
        assert!(updated_order.is_some());
        assert_eq!(updated_order.unwrap().display_quantity(), 50);

        // The price level should reflect the new quantity
        assert_eq!(price_level.display_quantity(), 50);
        assert_eq!(price_level.order_count(), 1);

        // Test updating non-existent order
        let update = OrderUpdate::UpdateQuantity {
            order_id: OrderId::from_u64(999),
            new_quantity: 50,
        };

        let result = price_level.update_order(update);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_update_order_update_price_and_quantity() {
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        let order = create_standard_order(1, 10000, 100);
        price_level.add_order(order);

        // Update both price and quantity with different price
        let update = OrderUpdate::UpdatePriceAndQuantity {
            order_id: OrderId::from_u64(1),
            new_price: 11000,
            new_quantity: 150,
        };

        let result = price_level.update_order(update);

        // The order should be removed from this price level (to be inserted in another price level)
        assert!(result.is_ok());
        let removed_order = result.unwrap();
        assert!(removed_order.is_some());
        assert_eq!(removed_order.unwrap().id(), OrderId::from_u64(1));

        // The price level should now be empty
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);

        // Test with same price but different quantity
        let order = create_standard_order(2, 10000, 100);
        price_level.add_order(order);

        let update = OrderUpdate::UpdatePriceAndQuantity {
            order_id: OrderId::from_u64(2),
            new_price: 10000,
            new_quantity: 150,
        };

        let result = price_level.update_order(update);

        // The order should be updated with the new quantity
        assert!(result.is_ok());
        let updated_order = result.unwrap();
        assert!(updated_order.is_some());
        assert_eq!(updated_order.unwrap().display_quantity(), 150);

        // The price level should reflect the new quantity
        assert_eq!(price_level.display_quantity(), 150);
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    fn test_update_order_replace() {
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        let order = create_standard_order(1, 10000, 100);
        price_level.add_order(order);

        // Replace with different price
        let update = OrderUpdate::Replace {
            order_id: OrderId::from_u64(1),
            price: 11000,
            quantity: 150,
            side: Side::Buy,
        };

        let result = price_level.update_order(update);

        // The order should be removed from this price level (to be inserted in another price level)
        assert!(result.is_ok());
        let removed_order = result.unwrap();
        assert!(removed_order.is_some());
        assert_eq!(removed_order.unwrap().id(), OrderId::from_u64(1));

        // The price level should now be empty
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);

        // Test with same price but different quantity
        let order = create_standard_order(2, 10000, 100);
        price_level.add_order(order);

        let update = OrderUpdate::Replace {
            order_id: OrderId::from_u64(2),
            price: 10000,
            quantity: 150,
            side: Side::Buy,
        };

        let result = price_level.update_order(update);

        // The order should be updated with the new quantity
        assert!(result.is_ok());
        let updated_order = result.unwrap();
        assert!(updated_order.is_some());
        assert_eq!(updated_order.unwrap().display_quantity(), 150);

        // The price level should reflect the new quantity
        assert_eq!(price_level.display_quantity(), 150);
        assert_eq!(price_level.order_count(), 1);
    }

    // Test the From<&PriceLevel> implementation for PriceLevelData
    #[test]
    fn test_price_level_data_from_price_level() {
        let mut price_level = PriceLevel::new(10000);

        // Add some orders
        price_level.add_order(create_standard_order(1, 10000, 100));
        price_level.add_order(create_standard_order(2, 10000, 50));

        // Convert to PriceLevelData
        let data: PriceLevelData = (&price_level).into();

        // Verify data fields
        assert_eq!(data.price, 10000);
        assert_eq!(data.display_quantity, 150); // 100 + 50
        assert_eq!(data.reserve_quantity, 0);
        assert_eq!(data.order_count, 2);
        assert_eq!(data.orders.len(), 2);

        // Verify order IDs
        let order_ids: Vec<OrderId> = data.orders.iter().map(|o| o.id()).collect();
        assert!(order_ids.contains(&OrderId::from_u64(1)));
        assert!(order_ids.contains(&OrderId::from_u64(2)));
    }

    // Test the TryFrom<PriceLevelData> implementation for PriceLevel
    #[test]
    fn test_price_level_try_from_price_level_data() {
        // Create PriceLevelData directly
        let data = PriceLevelData {
            price: 10000,
            display_quantity: 150,
            reserve_quantity: 0,
            order_count: 2,
            orders: vec![
                create_standard_order(1, 10000, 100),
                create_standard_order(2, 10000, 50),
            ],
        };

        // Convert to PriceLevel
        let result = PriceLevel::try_from(data);
        assert!(result.is_ok());

        let price_level = result.unwrap();

        // Verify price level properties
        assert_eq!(price_level.price(), 10000);
        assert_eq!(price_level.display_quantity(), 150);
        assert_eq!(price_level.reserve_quantity(), 0);
        assert_eq!(price_level.order_count(), 2);

        // Verify orders
        let orders = price_level.iter_orders();
        assert_eq!(orders.len(), 2);

        let order_ids: Vec<OrderId> = orders.iter().map(|o| o.id()).collect();
        assert!(order_ids.contains(&OrderId::from_u64(1)));
        assert!(order_ids.contains(&OrderId::from_u64(2)));
    }

    // Test Display implementation for PriceLevel
    #[test]
    fn test_price_level_display() {
        let mut price_level = PriceLevel::new(10000);
        price_level.add_order(create_standard_order(1, 10000, 100));

        let display_str = format!("{price_level}");

        // Verify the format
        assert!(display_str.starts_with("PriceLevel:price=10000;"));
        assert!(display_str.contains("display_quantity=100"));
        assert!(display_str.contains("reserve_quantity=0"));
        assert!(display_str.contains("order_count=1"));
        assert!(display_str.contains("orders=["));
        assert!(display_str.contains("Standard:id=00000000-0000-0001-0000-000000000000"));
    }

    // Test FromStr implementation for PriceLevel
    #[test]
    fn test_price_level_from_str() {
        let mut price_level = PriceLevel::new(10000);
        price_level.add_order(create_standard_order(1, 10000, 50));
        price_level.add_order(create_standard_order(2, 10000, 75));
        price_level.add_order(create_good_till_date_order(3, 10000, 100, 1617000000000));
        price_level.add_order(create_reserve_order(4, 10000, 100, 100, 20, true, None));
        price_level.add_order(create_iceberg_order(5, 10000, 50, 100));

        let input = "PriceLevel:price=10000;display_quantity=375;reserve_quantity=200;order_count=5;orders=[Standard:id=00000000-0000-0001-0000-000000000000;price=10000;display_quantity=50;side=BUY;timestamp=1616823000000;time_in_force=GTC,Standard:id=00000000-0000-0002-0000-000000000000;price=10000;display_quantity=75;side=BUY;timestamp=1616823000001;time_in_force=GTC,Standard:id=00000000-0000-0003-0000-000000000000;price=10000;display_quantity=100;side=BUY;timestamp=1616823000002;time_in_force=GTD-1617000000000,ReserveOrder:id=00000000-0000-0004-0000-000000000000;price=10000;display_quantity=100;reserve_quantity=100;side=SELL;timestamp=1616823000003;time_in_force=GTC;replenish_threshold=20;replenish_amount=None;auto_replenish=true,IcebergOrder:id=00000000-0000-0005-0000-000000000000;price=10000;display_quantity=50;reserve_quantity=100;side=SELL;timestamp=1616823000004;time_in_force=GTC]";
        let result = PriceLevel::from_str(input);

        if let Err(ref err) = result {
            error!("Error parsing PriceLevel: {:?}", err);
        }

        assert!(result.is_ok());

        let price_level = result.unwrap();

        // Verify price level properties
        assert_eq!(price_level.price(), 10000);
        assert_eq!(price_level.display_quantity(), 375);
        assert_eq!(price_level.reserve_quantity(), 200);
        assert_eq!(price_level.order_count(), 5);

        // Verify the order
        let orders = price_level.iter_orders();
        assert_eq!(orders.len(), 5);
        assert_eq!(orders[0].id(), OrderId::from_u64(1));
        assert_eq!(orders[0].price(), 10000);
        assert_eq!(orders[0].display_quantity(), 50);
    }

    // Test serialization and deserialization for PriceLevel
    #[test]
    fn test_price_level_serde() {
        let mut price_level = PriceLevel::new(10000);
        price_level.add_order(create_standard_order(1, 10000, 100));

        // Serialize to JSON
        let serialized = serde_json::to_string(&price_level).unwrap();

        // Verify the JSON structure
        assert!(serialized.contains("\"price\":10000"));
        assert!(serialized.contains("\"display_quantity\":100"));
        assert!(serialized.contains("\"reserve_quantity\":0"));
        assert!(serialized.contains("\"order_count\":1"));
        assert!(serialized.contains("\"orders\":"));

        // Deserialize back
        let deserialized: PriceLevel = serde_json::from_str(&serialized).unwrap();

        // Verify deserialized price level
        assert_eq!(deserialized.price(), 10000);
        assert_eq!(deserialized.display_quantity(), 100);
        assert_eq!(deserialized.reserve_quantity(), 0);
        assert_eq!(deserialized.order_count(), 1);

        // Verify the order in the deserialized price level
        let orders = deserialized.iter_orders();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].id(), OrderId::from_u64(1));
        assert_eq!(orders[0].price(), 10000);
        assert_eq!(orders[0].display_quantity(), 100);
    }

    // In price_level/level.rs test module or in a separate test file

    #[test]
    fn test_level_partial_match_remaining() {
        let mut price_level = PriceLevel::new(10000);
        let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction_id_generator = UuidGenerator::new(namespace);

        // Add orders with more quantity than we'll match
        price_level.add_order(create_standard_order(1, 10000, 200));

        // Match only part of what's available
        let match_result =
            price_level.match_order(100, OrderId::from_u64(999), &transaction_id_generator);

        assert_eq!(match_result.remaining_quantity, 0);
        assert!(match_result.is_complete);
        assert_eq!(price_level.display_quantity(), 100); // 200 - 100 = 100
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    fn test_level_update_price_different_price() {
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        price_level.add_order(create_standard_order(1, 10000, 100));

        // Update to a different price (should remove from this level)
        let result = price_level.update_order(OrderUpdate::UpdatePrice {
            order_id: OrderId::from_u64(1),
            new_price: 10100, // Different price
        });

        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
        assert_eq!(price_level.display_quantity(), 0);
        assert_eq!(price_level.order_count(), 0);
    }

    #[test]
    fn test_level_update_price_and_quantity_same_price() {
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        price_level.add_order(create_standard_order(1, 10000, 100));

        // Update the quantity but keep the same price
        let result = price_level.update_order(OrderUpdate::UpdatePriceAndQuantity {
            order_id: OrderId::from_u64(1),
            new_price: 10000, // Same price
            new_quantity: 150,
        });

        assert!(result.is_ok());
        let updated_order = result.unwrap().unwrap();
        assert_eq!(updated_order.display_quantity(), 150);
        assert_eq!(price_level.display_quantity(), 150);
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    fn test_serialize_deserialize_with_orders() {
        let mut price_level = PriceLevel::new(10000);

        // Add some orders
        price_level.add_order(create_standard_order(1, 10000, 100));
        price_level.add_order(create_iceberg_order(2, 10000, 50, 150));

        // Serialize to JSON
        let serialized = serde_json::to_string(&price_level).unwrap();

        // Deserialize back
        let deserialized: PriceLevel = serde_json::from_str(&serialized).unwrap();

        // Verify deserialized state matches original
        assert_eq!(deserialized.price(), price_level.price());
        assert_eq!(
            deserialized.display_quantity(),
            price_level.display_quantity()
        );
        assert_eq!(
            deserialized.reserve_quantity(),
            price_level.reserve_quantity()
        );
        assert_eq!(deserialized.order_count(), price_level.order_count());
    }

    #[test]
    fn test_price_level_update_price_same_value() {
        // Test lines 187-188
        let mut price_level = PriceLevel::new(10000);
        let order = Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 10000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };
        price_level.add_order(order);

        // Try to update price to the same value
        let update = OrderUpdate::UpdatePrice {
            order_id: OrderId::from_u64(1),
            new_price: 10000,
        };

        // This should return an error
        let result = price_level.update_order(update);
        assert!(result.is_err());
        match result {
            Err(PriceLevelError::InvalidOperation { message }) => {
                assert!(message.contains("Cannot update price to the same value"));
            }
            _ => panic!("Expected InvalidOperation error"),
        }
    }

    #[test]
    fn test_price_level_update_quantity_order_not_found() {
        // Test line 282
        let mut price_level = PriceLevel::new(10000);
        // No orders added

        // Try to update quantity of a non-existent order
        let update = OrderUpdate::UpdateQuantity {
            order_id: OrderId::from_u64(123),
            new_quantity: 20,
        };

        let result = price_level.update_order(update);
        // Should return Ok(None) when order not found
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_price_level_update_quantity_by_another_thread() {
        // Test lines 304-306, 308-309
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        let order = Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 10000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };
        price_level.add_order(order);

        // Set up a test that simulates order removal by another thread
        // This can be done by modifying the OrderQueue's internal state directly
        // or by simply testing the behavior of the update_quantity method when it returns None

        // For now, we'll just mock this behavior by ensuring the method handles
        // cases where an order is not found after initial check (order was found but removed)

        // First find the order to make sure it exists
        assert!(
            price_level
                .update_order(OrderUpdate::Cancel {
                    order_id: OrderId::from_u64(1)
                })
                .unwrap()
                .is_some()
        );

        // Now try to update it after it's been removed
        let update = OrderUpdate::UpdateQuantity {
            order_id: OrderId::from_u64(1),
            new_quantity: 20,
        };

        let result = price_level.update_order(update);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_price_level_update_quantity_increase() {
        // Test line 473
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        let order = Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 10000,
                display_quantity: 50,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };
        price_level.add_order(order);

        // Update to increase quantity (old visible < new visible)
        let update = OrderUpdate::UpdateQuantity {
            order_id: OrderId::from_u64(1),
            new_quantity: 100,
        };

        let result = price_level.update_order(update);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());

        // Verify quantity increased
        assert_eq!(price_level.display_quantity(), 100);
    }

    #[test]
    fn test_price_level_update_reserve_quantity() {
        // Test lines 488, 498
        let mut price_level = PriceLevel::new(10000);

        // Add an iceberg order with visible and hidden quantities
        let order = Order::IcebergOrder {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 10000,
                display_quantity: 50,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 150,
        };
        price_level.add_order(order);

        // Verify initial quantities
        assert_eq!(price_level.display_quantity(), 50);
        assert_eq!(price_level.reserve_quantity(), 150);

        // Create a new iceberg order with different quantities
        let new_order = Order::IcebergOrder {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 10000,
                display_quantity: 40,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 200,
        };

        // Test increasing hidden quantity
        let result = price_level.update_order(OrderUpdate::Cancel {
            order_id: OrderId::from_u64(1),
        });
        assert!(result.is_ok());
        price_level.add_order(new_order);

        // Verify both visible and hidden quantities were updated
        assert_eq!(price_level.display_quantity(), 40);
        assert_eq!(price_level.reserve_quantity(), 200);
    }

    #[test]
    fn test_price_level_update_price_and_quantity_same_price() {
        // Test line 510
        let mut price_level = PriceLevel::new(10000);

        // Add an order
        let order = Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 10000,
                display_quantity: 50,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };
        price_level.add_order(order);

        // Update both price and quantity with same price
        let update = OrderUpdate::UpdatePriceAndQuantity {
            order_id: OrderId::from_u64(1),
            new_price: 10000, // Same price
            new_quantity: 100,
        };

        let result = price_level.update_order(update);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());

        // Verify quantity was updated but price remained the same
        assert_eq!(price_level.display_quantity(), 100);
        assert_eq!(price_level.price(), 10000);
    }

    #[test]
    fn test_price_level_from_price_level_data_conversion() {
        // Test lines 521-523, 527, 537, 558-560, 562-564, 566-568, 607

        // Create a price level
        let mut price_level = PriceLevel::new(10000);

        // Add some orders
        let order1 = Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 10000,
                display_quantity: 50,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };
        price_level.add_order(order1);

        let order2 = Order::<()>::IcebergOrder {
            common: OrderCommon {
                id: OrderId::from_u64(2),
                price: 10000,
                display_quantity: 30,
                side: Side::Buy,
                timestamp: 1616823000001,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 70,
        };
        price_level.add_order(order2);

        // Convert to PriceLevelData
        let data: PriceLevelData = (&price_level).into();

        // Verify data
        assert_eq!(data.price, 10000);
        assert_eq!(data.display_quantity, 80); // 50 + 30
        assert_eq!(data.reserve_quantity, 70);
        assert_eq!(data.order_count, 2);
        assert_eq!(data.orders.len(), 2);

        // Convert back to PriceLevel
        let result = PriceLevel::try_from(data);
        assert!(result.is_ok());

        // Verify converted price level
        let converted_level = result.unwrap();
        assert_eq!(converted_level.price(), 10000);
        assert_eq!(converted_level.display_quantity(), 80);
        assert_eq!(converted_level.reserve_quantity(), 70);
        assert_eq!(converted_level.order_count(), 2);

        // Test display implementation
        let display_string = price_level.to_string();
        assert!(display_string.starts_with("PriceLevel:price=10000;"));
        assert!(display_string.contains("display_quantity=80"));
        assert!(display_string.contains("reserve_quantity=70"));
        assert!(display_string.contains("order_count=2"));

        // Test serialization
        let serialized = serde_json::to_string(&price_level).unwrap();
        assert!(serialized.contains("\"price\":10000"));
        assert!(serialized.contains("\"display_quantity\":80"));
        assert!(serialized.contains("\"reserve_quantity\":70"));
        assert!(serialized.contains("\"order_count\":2"));

        // Test deserialization
        let deserialized: PriceLevel = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.price(), 10000);
        assert_eq!(deserialized.display_quantity(), 80);
        assert_eq!(deserialized.reserve_quantity(), 70);
        assert_eq!(deserialized.order_count(), 2);
    }
}

#[cfg(test)]
mod tests_eq {
    use crate::PriceLevel;

    #[test]
    fn test_price_level_partial_eq() {
        // Create two price levels with the same price
        let price_level1 = PriceLevel::new(10000);
        let price_level2 = PriceLevel::new(10000);

        // Create a price level with a different price
        let price_level3 = PriceLevel::new(10001);

        // Test equality
        assert_eq!(price_level1, price_level2);

        // Test inequality
        assert_ne!(price_level1, price_level3);
        assert_ne!(price_level2, price_level3);
    }

    #[test]
    fn test_price_level_eq() {
        // Test Eq trait (reflexivity, symmetry, transitivity)
        let price_level1 = PriceLevel::new(10000);
        let price_level2 = PriceLevel::new(10000);
        let price_level3 = PriceLevel::new(10000);

        // Reflexivity: a == a
        assert_eq!(price_level1, price_level1);

        // Symmetry: if a == b then b == a
        assert_eq!(price_level1, price_level2);
        assert_eq!(price_level2, price_level1);

        // Transitivity: if a == b and b == c then a == c
        assert_eq!(price_level1, price_level2);
        assert_eq!(price_level2, price_level3);
        assert_eq!(price_level1, price_level3);
    }

    #[test]
    fn test_price_level_partial_ord() {
        let price_level1 = PriceLevel::new(10000);
        let price_level2 = PriceLevel::new(10500);
        let price_level3 = PriceLevel::new(9500);

        // Test comparisons
        assert!(price_level1 < price_level2);
        assert!(price_level3 < price_level1);
        assert!(price_level3 < price_level2);

        assert!(price_level2 > price_level1);
        assert!(price_level1 > price_level3);
        assert!(price_level2 > price_level3);

        assert!(price_level1 <= price_level2);
        assert!(price_level1 <= price_level1); // Equality case

        assert!(price_level2 >= price_level1);
        assert!(price_level1 >= price_level1); // Equality case
    }

    #[test]
    fn test_price_level_ord() {
        // Create some price levels
        let price_level1 = PriceLevel::new(9000);
        let price_level2 = PriceLevel::new(10000);
        let price_level3 = PriceLevel::new(11000);

        // Create a vector of price level references
        let mut price_level_refs = [&price_level3, &price_level1, &price_level2];

        // Sort the vector - this uses the Ord implementation
        price_level_refs.sort();

        // Verify the sorting order (ascending by price)
        assert_eq!(price_level_refs[0].price(), 9000);
        assert_eq!(price_level_refs[1].price(), 10000);
        assert_eq!(price_level_refs[2].price(), 11000);

        // Test the comparison methods directly
        assert_eq!(price_level1.cmp(&price_level2), std::cmp::Ordering::Less);
        assert_eq!(price_level2.cmp(&price_level1), std::cmp::Ordering::Greater);
        assert_eq!(price_level2.cmp(&price_level2), std::cmp::Ordering::Equal);
    }
}
