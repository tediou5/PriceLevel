use crate::errors::PriceLevelError;
use crate::order::{OrderId, Order};
use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::fmt::Display;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

/// A thread-safe queue of orders with specialized operations
#[derive(Debug)]
pub struct OrderQueue {
    /// A map of order IDs to orders for quick lookups
    orders: DashMap<OrderId, Arc<Order<()>>>,
    /// A queue of order IDs to maintain FIFO order
    order_ids: SegQueue<OrderId>,
}

impl OrderQueue {
    /// Create a new empty order queue
    pub fn new() -> Self {
        Self {
            orders: DashMap::new(),
            order_ids: SegQueue::new(),
        }
    }

    /// Add an order to the queue
    pub fn push(&self, order: Arc<Order<()>>) {
        let order_id = order.id();
        self.orders.insert(order_id, order);
        self.order_ids.push(order_id);
    }

    /// Attempt to pop an order from the queue
    pub fn pop(&self) -> Option<Arc<Order<()>>> {
        loop {
            if let Some(order_id) = self.order_ids.pop() {
                // If the order was removed, pop will return None, but the ID was in the queue.
                // In this case, we loop and try to get the next one.
                if let Some((_, order)) = self.orders.remove(&order_id) {
                    return Some(order);
                }
            } else {
                return None; // Queue is empty
            }
        }
    }

    /// Search for an order with the given ID. O(1) operation.
    pub fn find(&self, order_id: OrderId) -> Option<Arc<Order<()>>> {
        self.orders.get(&order_id).map(|o| o.value().clone())
    }

    /// Remove an order with the given ID
    /// Returns the removed order if found. O(1) for the map, but the ID remains in the queue.
    pub fn remove(&self, order_id: OrderId) -> Option<Arc<Order<()>>> {
        self.orders.remove(&order_id).map(|(_, order)| order)
    }

    /// Convert the queue to a vector (for snapshots)
    pub fn to_vec(&self) -> Vec<Arc<Order<()>>> {
        let mut orders: Vec<Arc<Order<()>>> =
            self.orders.iter().map(|o| o.value().clone()).collect();
        orders.sort_by_key(|o| o.timestamp());
        orders
    }

    /// Creates a new `OrderQueue` instance and populates it with orders from the provided vector.
    ///
    /// This function takes ownership of a vector of order references (wrapped in `Arc`) and constructs
    /// a new `OrderQueue` by iteratively pushing each order into the queue. The resulting queue
    /// maintains the insertion order of the original vector.
    ///
    /// # Parameters
    ///
    /// * `orders` - A vector of atomic reference counted (`Arc`) order instances representing
    ///   the orders to be added to the new queue.
    ///
    /// # Returns
    ///
    /// A new `OrderQueue` instance containing all the orders from the input vector.
    ///
    #[allow(dead_code)]
    pub fn from_vec(orders: Vec<Arc<Order<()>>>) -> Self {
        let queue = OrderQueue::new();
        for order in orders {
            queue.push(order);
        }
        queue
    }

    /// Check if the queue is empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Returns the number of orders currently in the queue.
    ///
    /// # Returns
    ///
    /// * `usize` - The total count of orders in the queue.
    ///
    pub fn len(&self) -> usize {
        self.orders.len()
    }
}

impl Default for OrderQueue {
    fn default() -> Self {
        Self::new()
    }
}
// Implement serialization for OrderQueue
impl Serialize for OrderQueue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for order_entry in self.orders.iter() {
            seq.serialize_element(order_entry.value().as_ref())?;
        }
        seq.end()
    }
}

impl FromStr for OrderQueue {
    type Err = PriceLevelError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("OrderQueue:orders=[") || !s.ends_with(']') {
            return Err(PriceLevelError::ParseError {
                message: "Invalid format".to_string(),
            });
        }

        let content = &s["OrderQueue:orders=[".len()..s.len() - 1];
        let queue = OrderQueue::new();

        if !content.is_empty() {
            for order_str in content.split(',') {
                let order =
                    Order::from_str(order_str).map_err(|e| PriceLevelError::ParseError {
                        message: format!("Order parse error: {e}"),
                    })?;
                queue.push(Arc::new(order));
            }
        }

        Ok(queue)
    }
}

impl Display for OrderQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let orders_str: Vec<String> = self.to_vec().iter().map(|o| o.to_string()).collect();
        write!(f, "OrderQueue:orders=[{}]", orders_str.join(","))
    }
}

impl From<Vec<Arc<Order<()>>>> for OrderQueue {
    fn from(orders: Vec<Arc<Order<()>>>) -> Self {
        let queue = OrderQueue::new();
        for order in orders {
            queue.push(order);
        }
        queue
    }
}

// Custom visitor for deserializing OrderQueue
struct OrderQueueVisitor {
    marker: PhantomData<fn() -> OrderQueue>,
}

impl OrderQueueVisitor {
    fn new() -> Self {
        OrderQueueVisitor {
            marker: PhantomData,
        }
    }
}

impl<'de> Visitor<'de> for OrderQueueVisitor {
    type Value = OrderQueue;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sequence of orders")
    }

    fn visit_seq<V>(self, mut seq: V) -> Result<OrderQueue, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let queue = OrderQueue::new();

        // Deserialize each order and add it to the queue
        while let Some(order) = seq.next_element::<Order<()>>()? {
            queue.push(Arc::new(order));
        }

        Ok(queue)
    }
}

// Implement deserialization for OrderQueue
impl<'de> Deserialize<'de> for OrderQueue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize as a sequence of orders
        deserializer.deserialize_seq(OrderQueueVisitor::new())

        // Alternative approach: Deserialize as OrderQueueData first, then convert
        // let data = OrderQueueData::deserialize(deserializer)?;
        // let queue = OrderQueue::new();
        // for order in data.orders {
        //     queue.push(Arc::new(order));
        // }
        // Ok(queue)
    }
}

#[cfg(test)]
mod tests {
    use crate::order::{OrderCommon, OrderId, Order, Side, TimeInForce};
    use crate::price_level::order_queue::OrderQueue;
    use std::str::FromStr;
    use std::sync::Arc;
    use tracing::info;

    fn create_test_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                price,
                display_quantity: quantity,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        }
    }

    #[test]
    fn test_display() {
        let queue = OrderQueue::new();
        queue.push(Arc::new(create_test_order(1, 1000, 10)));
        queue.push(Arc::new(create_test_order(2, 1100, 20)));

        let display_string = queue.to_string();
        info!("Display: {}", display_string);

        assert!(display_string.starts_with("OrderQueue:orders=["));
        assert!(display_string.contains("id=00000000-0000-0001-0000-000000000000"));
        assert!(display_string.contains("id=00000000-0000-0002-0000-000000000000"));
        assert!(display_string.contains("price=1000"));
        assert!(display_string.contains("price=1100"));
    }

    #[test]
    fn test_from_str() {
        // Create a queue directly for consistency check
        let queue = OrderQueue::new();
        queue.push(Arc::new(create_test_order(1, 1000, 10)));
        queue.push(Arc::new(create_test_order(2, 1100, 20)));

        // Get the display string
        let display_string = queue.to_string();

        // Verify display string format
        assert!(display_string.starts_with("OrderQueue:orders=["));
        assert!(display_string.contains("id=00000000-0000-0001-0000-000000000000"));
        assert!(display_string.contains("id=00000000-0000-0002-0000-000000000000"));
        assert!(display_string.contains("price=1000"));
        assert!(display_string.contains("price=1100"));

        // Example input string format (manually constructed to match expected format)
        let input = "OrderQueue:orders=[Standard:id=00000000-0000-0001-0000-000000000000;price=1000;display_quantity=10;side=BUY;timestamp=1616823000000;time_in_force=GTC,Standard:id=00000000-0000-0002-0000-000000000000;price=1100;display_quantity=20;side=BUY;timestamp=1616823000000;time_in_force=GTC]";

        // Try parsing
        let parsed_queue = match OrderQueue::from_str(input) {
            Ok(q) => q,
            Err(e) => {
                info!("Parse error: {:?}", e);
                info!("Input string: {}", input);
                panic!("Failed to parse OrderQueue from string");
            }
        };

        // Verify the parsed queue
        assert!(!parsed_queue.is_empty());
        let orders = parsed_queue.to_vec();

        // Should have both orders
        assert_eq!(orders.len(), 2, "Expected 2 orders in parsed queue");

        // Verify individual orders (order might not be preserved)
        let has_order1 = orders.iter().any(|o| {
            o.id() == OrderId::from_u64(1) && o.price() == 1000 && o.display_quantity() == 10
        });
        let has_order2 = orders.iter().any(|o| {
            o.id() == OrderId::from_u64(2) && o.price() == 1100 && o.display_quantity() == 20
        });

        assert!(has_order1, "First order not found or incorrect");
        assert!(has_order2, "Second order not found or incorrect");

        // Test round-trip parsing
        let round_trip_queue = OrderQueue::from_str(&display_string).unwrap();
        let round_trip_orders = round_trip_queue.to_vec();

        assert_eq!(
            round_trip_orders.len(),
            2,
            "Round-trip parsing should preserve order count"
        );

        let round_trip_has_order1 = round_trip_orders.iter().any(|o| {
            o.id() == OrderId::from_u64(1) && o.price() == 1000 && o.display_quantity() == 10
        });
        let round_trip_has_order2 = round_trip_orders.iter().any(|o| {
            o.id() == OrderId::from_u64(2) && o.price() == 1100 && o.display_quantity() == 20
        });

        assert!(
            round_trip_has_order1,
            "First order not preserved in round-trip"
        );
        assert!(
            round_trip_has_order2,
            "Second order not preserved in round-trip"
        );
    }

    #[test]
    fn test_serialize_deserialize() {
        let queue = OrderQueue::new();
        queue.push(Arc::new(create_test_order(1, 1000, 10)));
        queue.push(Arc::new(create_test_order(2, 1100, 20)));

        // Serialize to JSON
        let serialized = serde_json::to_string(&queue).unwrap();
        info!("Serialized: {}", serialized);

        // Deserialize back
        let deserialized: OrderQueue = serde_json::from_str(&serialized).unwrap();

        // Verify
        let original_orders = queue.to_vec();
        let deserialized_orders = deserialized.to_vec();

        assert_eq!(original_orders.len(), deserialized_orders.len());

        // Since the order of orders might not be preserved, compare individual orders
        for order in original_orders {
            let found = deserialized_orders.iter().any(|o| o.id() == order.id());
            assert!(
                found,
                "Order with ID {} not found after deserialization",
                order.id()
            );
        }
    }

    #[test]
    fn test_round_trip() {
        let queue = OrderQueue::new();
        queue.push(Arc::new(create_test_order(1, 1000, 10)));

        // Convert to string
        let string_rep = queue.to_string();

        // Parse back from string
        let parsed_queue = match OrderQueue::from_str(&string_rep) {
            Ok(q) => q,
            Err(e) => {
                info!("Parse error: {:?}", e);
                panic!("Failed to parse: {string_rep}");
            }
        };

        // Verify
        let original_orders = queue.to_vec();
        let parsed_orders = parsed_queue.to_vec();

        assert_eq!(original_orders.len(), parsed_orders.len());
        assert_eq!(original_orders[0].id(), parsed_orders[0].id());
        assert_eq!(original_orders[0].price(), parsed_orders[0].price());
    }

    // In price_level/order_queue.rs test module or in a separate test file

    #[test]
    fn test_order_queue_to_vec_empty() {
        let queue = OrderQueue::new();

        // test_to_vec on empty queue
        let vec = queue.to_vec();
        assert!(vec.is_empty());

        // Verify queue is still empty after to_vec
        assert!(queue.is_empty());
    }

    #[test]
    fn test_order_queue_from_str_complex() {
        // Test with a complex order string format
        let complex_order = "Standard:id=00000000-0000-0001-0000-000000000000;price=10000;display_quantity=100;side=BUY;timestamp=1616823000000;time_in_force=GTD-1617000000000";

        let input = format!("OrderQueue:orders=[{complex_order}]");
        let queue = OrderQueue::from_str(&input).unwrap();

        assert_eq!(queue.len(), 1);

        // Verify the order's details
        let order = &queue.to_vec()[0];

        if let Order::<()>::Standard {
            common:
                OrderCommon {
                    id,
                    price,
                    display_quantity: quantity,
                    time_in_force,
                    ..
                },
        } = **order
        {
            assert_eq!(id, OrderId::from_u64(1));
            assert_eq!(price, 10000);
            assert_eq!(quantity, 100);
            assert!(matches!(time_in_force, TimeInForce::Gtd(1617000000000)));
        } else {
            panic!("Expected Standard order");
        }
    }

    #[test]
    fn test_order_queue_from_str_invalid_order() {
        // Test with an invalid order format
        let input = "OrderQueue:orders=[InvalidOrder:id=1]";
        let result = OrderQueue::from_str(input);

        assert!(result.is_err());
    }

    #[test]
    fn test_order_queue_serialization() {
        fn create_standard_order(id: u64, price: u64, quantity: u64) -> Order<()> {
            Order::Standard {
                common: OrderCommon {
                    id: OrderId::from_u64(id),
                    price,
                    display_quantity: quantity,
                    side: Side::Buy,
                    timestamp: 1616823000000,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
            }
        }
        let queue = OrderQueue::new();

        // Add an order
        let order = create_standard_order(1, 10000, 100);
        queue.push(Arc::new(order));

        // Serialize
        let serialized = serde_json::to_string(&queue).unwrap();

        // Check that it contains the expected order data
        assert!(serialized.contains("\"Standard\""));
        assert!(serialized.contains("\"id\":\"00000000-0000-0001-0000-000000000000\""));
        assert!(serialized.contains("\"price\":10000"));
        assert!(serialized.contains("\"display_quantity\":100"));

        // Deserialize and verify
        let deserialized: OrderQueue = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.len(), 1);

        let deserialized_order = &deserialized.to_vec()[0];

        if let Order::Standard {
            common:
                OrderCommon {
                    id,
                    price,
                    display_quantity: quantity,
                    ..
                },
        } = **deserialized_order
        {
            assert_eq!(id, OrderId::from_u64(1));
            assert_eq!(price, 10000);
            assert_eq!(quantity, 100);
        } else {
            panic!("Expected Standard order");
        }
    }

    #[test]
    fn test_order_queue_empty_check() {
        // Test lines 123-124
        let queue = OrderQueue::new();

        // Queue should be empty initially
        assert!(queue.is_empty());

        // Add an order and check again
        let order = Order::Standard {
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
        queue.push(Arc::new(order));

        // Queue should not be empty after adding an order
        assert!(!queue.is_empty());

        // Remove the order and check again
        let _ = queue.pop();
        assert!(queue.is_empty());

        // Push the order back and then try a different approach to check emptiness
        let order2 = Order::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(2),
                price: 1000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };
        queue.push(Arc::new(order2));
        assert!(!queue.is_empty());
    }

    #[test]
    fn test_order_queue_from_vec() {
        // Test lines 170, 178
        // Create a vector of orders
        let order1 = Arc::new(Order::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 1000,
                display_quantity: 10,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        });

        let order2 = Arc::new(Order::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(2),
                price: 1000,
                display_quantity: 20,
                side: Side::Buy,
                timestamp: 1616823000001,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        });

        let orders = vec![order1.clone(), order2.clone()];

        // Create a queue from the vector
        let queue = OrderQueue::from_vec(orders.clone());

        // Verify the queue contains the orders
        assert_eq!(queue.to_vec().len(), 2);
        assert!(queue.to_vec().contains(&order1));
        assert!(queue.to_vec().contains(&order2));

        // Test the From implementation
        let queue_from_trait: OrderQueue = orders.clone().into();
        assert_eq!(queue_from_trait.to_vec().len(), 2);

        // Test the Into implementation
        let orders_from_queue: Vec<Arc<Order<()>>> = queue.into();
        assert_eq!(orders_from_queue.len(), 2);
        assert!(orders_from_queue.contains(&order1));
        assert!(orders_from_queue.contains(&order2));
    }

    #[test]
    fn test_order_queue_from_str_parsing_with_complex_content() {
        // Test lines 196-198, 200-202, 241, 266-267

        // Create a complex string with nested delimiters
        let complex_input = "OrderQueue:orders=[Standard:id=00000000-0000-0001-0000-000000000000;price=1000;display_quantity=10;side=BUY;timestamp=1616823000000;time_in_force=GTC,IcebergOrder:id=00000000-0000-0002-0000-000000000000;price=1000;display_quantity=5;reserve_quantity=15;side=SELL;timestamp=1616823000001;time_in_force=GTC]";

        // Parse the complex input
        let result = OrderQueue::from_str(complex_input);
        assert!(result.is_ok());

        let queue = result.unwrap();
        assert_eq!(queue.to_vec().len(), 2);

        // Verify the parsed orders have the expected IDs
        let order_ids: Vec<OrderId> = queue.to_vec().iter().map(|order| order.id()).collect();
        assert!(order_ids.contains(&OrderId::from_u64(1)));
        assert!(order_ids.contains(&OrderId::from_u64(2)));

        // Test parsing with empty orders
        let empty_orders = "OrderQueue:orders=[]";
        let result = OrderQueue::from_str(empty_orders);
        assert!(result.is_ok());
        let queue = result.unwrap();
        assert!(queue.is_empty());

        // Test parsing with invalid format (no "OrderQueue:" prefix)
        let invalid_input = "orders=[Standard:id=1;price=1000;quantity=10;side=BUY;timestamp=1616823000000;time_in_force=GTC]";
        let result = OrderQueue::from_str(invalid_input);
        assert!(result.is_err());

        // Test parsing with malformed content (missing closing bracket)
        let malformed_input = "OrderQueue:orders=[Standard:id=00000000-0000-0001-0000-000000000000;price=1000;quantity=10;side=BUY;timestamp=1616823000000;time_in_force=GTC";
        let result = OrderQueue::from_str(malformed_input);
        assert!(result.is_err());

        // Test parsing with invalid order type
        let invalid_order = "OrderQueue:orders=[InvalidOrder:id=00000000-0000-0001-0000-000000000000;price=1000;quantity=10;side=BUY;timestamp=1616823000000;time_in_force=GTC]";
        let result = OrderQueue::from_str(invalid_order);
        assert!(result.is_err());
    }

    #[test]
    fn test_order_queue_serialization_deserialization() {
        // Create a queue with orders
        let queue = OrderQueue::new();

        let order1 = Order::Standard {
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

        let order2 = Order::IcebergOrder {
            common: OrderCommon {
                id: OrderId::from_u64(2),
                price: 1000,
                display_quantity: 5,
                side: Side::Sell,
                timestamp: 1616823000001,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 15,
        };

        queue.push(Arc::new(order1));
        queue.push(Arc::new(order2));

        // Serialize the queue
        let serialized = serde_json::to_string(&queue).unwrap();

        // Verify the serialized format contains the orders
        assert!(serialized.contains("\"id\":\"00000000-0000-0001-0000-000000000000\""));
        assert!(serialized.contains("\"id\":\"00000000-0000-0002-0000-000000000000\""));

        // Deserialize back to OrderQueue
        let deserialized: OrderQueue = serde_json::from_str(&serialized).unwrap();

        // Verify the deserialized queue has the same orders
        assert_eq!(deserialized.to_vec().len(), 2);

        // Verify the order IDs
        let order_ids: Vec<OrderId> = deserialized
            .to_vec()
            .iter()
            .map(|order| order.id())
            .collect();
        assert!(order_ids.contains(&OrderId::from_u64(1)));
        assert!(order_ids.contains(&OrderId::from_u64(2)));
    }
}
