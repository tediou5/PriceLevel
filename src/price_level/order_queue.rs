use crate::order::{Order, OrderId};
use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;
use std::str::FromStr;

/// A single-threaded queue of orders with efficient operations
#[derive(Debug)]
pub struct OrderQueue {
    /// A map of order IDs to orders for quick lookups
    orders: HashMap<OrderId, Rc<Order<()>>>,
    /// A queue of order IDs to maintain FIFO order
    order_ids: VecDeque<OrderId>,
}

impl OrderQueue {
    /// Create a new empty order queue
    pub fn new() -> Self {
        Self {
            orders: HashMap::new(),
            order_ids: VecDeque::new(),
        }
    }

    /// Add an order to the queue
    pub fn push(&mut self, order: Rc<Order<()>>) {
        let order_id = order.id();
        self.orders.insert(order_id, order);
        self.order_ids.push_back(order_id);
    }

    /// Attempt to pop an order from the queue
    pub fn pop(&mut self) -> Option<Rc<Order<()>>> {
        while let Some(order_id) = self.order_ids.pop_front() {
            if let Some(order) = self.orders.remove(&order_id) {
                return Some(order);
            }
            // If the order was removed but ID was still in queue, continue to next
        }
        None
    }

    /// Find an order by ID
    pub fn find(&self, order_id: &OrderId) -> Option<Rc<Order<()>>> {
        self.orders.get(order_id).cloned()
    }

    /// Remove an order by ID
    pub fn remove(&mut self, order_id: &OrderId) -> Option<Rc<Order<()>>> {
        self.orders.remove(order_id)
    }

    /// Convert queue to vector (for iteration)
    pub fn to_vec(&self) -> Vec<Rc<Order<()>>> {
        self.order_ids
            .iter()
            .filter_map(|id| self.orders.get(id).cloned())
            .collect()
    }

    /// Create queue from vector of orders
    pub fn from_vec(orders: Vec<Rc<Order<()>>>) -> Self {
        let mut order_queue = Self::new();
        for order in orders {
            order_queue.push(order);
        }
        order_queue
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Get the number of orders in the queue
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    /// Iterator over orders
    pub fn iter(&self) -> impl Iterator<Item = &Rc<Order<()>>> {
        self.order_ids
            .iter()
            .filter_map(move |id| self.orders.get(id))
    }
}

impl Default for OrderQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl Serialize for OrderQueue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let orders = self.to_vec();
        let mut seq = serializer.serialize_seq(Some(orders.len()))?;
        for order in orders {
            seq.serialize_element(&*order)?;
        }
        seq.end()
    }
}

impl FromStr for OrderQueue {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let orders: Vec<Order<()>> = serde_json::from_str(s)?;
        let rc_orders: Vec<Rc<Order<()>>> = orders.into_iter().map(Rc::new).collect();
        Ok(Self::from_vec(rc_orders))
    }
}

impl fmt::Display for OrderQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let orders = self.to_vec();
        let order_values: Vec<Order<()>> = orders.iter().map(|rc| **rc).collect();
        let json = serde_json::to_string(&order_values).map_err(|_| fmt::Error)?;
        write!(f, "{}", json)
    }
}

impl From<Vec<Rc<Order<()>>>> for OrderQueue {
    fn from(orders: Vec<Rc<Order<()>>>) -> Self {
        Self::from_vec(orders)
    }
}

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

    fn visit_seq<V>(self, mut seq: V) -> Result<Self::Value, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let mut orders = Vec::new();
        while let Some(order) = seq.next_element::<Order<()>>()? {
            orders.push(Rc::new(order));
        }
        Ok(OrderQueue::from_vec(orders))
    }
}

impl<'de> Deserialize<'de> for OrderQueue {
    fn deserialize<D>(deserializer: D) -> Result<OrderQueue, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(OrderQueueVisitor::new())
    }
}

#[cfg(test)]
mod tests {
    use crate::order::{Order, OrderCommon, OrderId, Side, TimeInForce};
    use crate::price_level::order_queue::OrderQueue;
    use std::rc::Rc;
    use std::str::FromStr;

    fn create_test_order(id: u64, price: u64, quantity: u64) -> Order<()> {
        Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(id),
                side: Side::Buy,
                price,
                display_quantity: quantity,
                time_in_force: TimeInForce::Gtc,
                timestamp: 0,
                extra_fields: (),
            },
        }
    }

    #[test]
    fn test_display() {
        let mut queue = OrderQueue::new();
        let order1 = Rc::new(create_test_order(1, 100, 10));
        let order2 = Rc::new(create_test_order(2, 101, 20));

        queue.push(order1);
        queue.push(order2);

        let display_str = queue.to_string();
        assert!(!display_str.is_empty());
        assert!(display_str.contains("100"));
        assert!(display_str.contains("101"));
    }

    #[test]
    fn test_from_str() {
        let json_str = r#"[
            {
                "Standard": {
                    "id": "00000000-0000-0001-0000-000000000000",
                    "price": 100,
                    "display_quantity": 10,
                    "side": "BUY",
                    "timestamp": 0,
                    "time_in_force": "GTC",
                    "extra_fields": null
                }
            },
            {
                "Standard": {
                    "id": "00000000-0000-0002-0000-000000000000",
                    "price": 101,
                    "display_quantity": 20,
                    "side": "SELL",
                    "timestamp": 0,
                    "time_in_force": "GTC",
                    "extra_fields": null
                }
            }
        ]"#;

        let queue = OrderQueue::from_str(json_str).unwrap();
        assert_eq!(queue.len(), 2);

        let orders = queue.to_vec();
        assert_eq!(orders[0].price(), 100);
        assert_eq!(orders[0].display_quantity(), 10);
        assert_eq!(orders[1].price(), 101);
        assert_eq!(orders[1].display_quantity(), 20);
    }

    #[test]
    fn test_serialize_deserialize() {
        let mut original_queue = OrderQueue::new();
        let order1 = Rc::new(create_test_order(1, 100, 10));
        let order2 = Rc::new(create_test_order(2, 101, 20));

        original_queue.push(order1);
        original_queue.push(order2);

        let serialized = serde_json::to_string(&original_queue).unwrap();
        let deserialized: OrderQueue = serde_json::from_str(&serialized).unwrap();

        assert_eq!(original_queue.len(), deserialized.len());

        let original_orders = original_queue.to_vec();
        let deserialized_orders = deserialized.to_vec();

        assert_eq!(original_orders[0].price(), deserialized_orders[0].price());
        assert_eq!(original_orders[1].price(), deserialized_orders[1].price());
    }

    #[test]
    fn test_round_trip() {
        let mut original_queue = OrderQueue::new();
        let order = Rc::new(create_test_order(1, 100, 10));
        original_queue.push(order);

        let display_str = original_queue.to_string();
        let parsed_queue = OrderQueue::from_str(&display_str).unwrap();

        assert_eq!(original_queue.len(), parsed_queue.len());

        let original_orders = original_queue.to_vec();
        let parsed_orders = parsed_queue.to_vec();

        assert_eq!(original_orders[0].price(), parsed_orders[0].price());
        assert_eq!(
            original_orders[0].display_quantity(),
            parsed_orders[0].display_quantity()
        );
    }

    #[test]
    fn test_order_queue_basic_operations() {
        let mut queue = OrderQueue::new();
        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        let order1 = Rc::new(create_test_order(1, 100, 10));
        let order2 = Rc::new(create_test_order(2, 101, 20));
        let order1_id = order1.id();
        let order2_id = order2.id();

        queue.push(order1.clone());
        queue.push(order2.clone());

        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 2);

        // Test find
        let found_order = queue.find(&order1_id).unwrap();
        assert_eq!(found_order.price(), 100);

        // Test pop (FIFO order)
        let popped = queue.pop().unwrap();
        assert_eq!(popped.price(), 100);
        assert_eq!(queue.len(), 1);

        // Test remove
        let removed = queue.remove(&order2_id).unwrap();
        assert_eq!(removed.price(), 101);
        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_order_queue_to_vec_empty() {
        let queue = OrderQueue::new();
        let orders = queue.to_vec();
        assert!(orders.is_empty());
    }

    #[test]
    fn test_order_queue_from_str_invalid_order() {
        let invalid_json = r#"[{"invalid": "order"}]"#;
        let result = OrderQueue::from_str(invalid_json);
        assert!(result.is_err());
    }

    #[test]
    fn test_order_queue_from_vec() {
        let order1 = Rc::new(create_test_order(1, 100, 10));
        let order2 = Rc::new(create_test_order(2, 101, 20));
        let orders = vec![order1.clone(), order2.clone()];

        let queue = OrderQueue::from_vec(orders);
        assert_eq!(queue.len(), 2);

        let queue_orders = queue.to_vec();
        assert_eq!(queue_orders[0].price(), 100);
        assert_eq!(queue_orders[1].price(), 101);
    }

    #[test]
    fn test_order_queue_iter() {
        let mut queue = OrderQueue::new();
        let order1 = Rc::new(create_test_order(1, 100, 10));
        let order2 = Rc::new(create_test_order(2, 101, 20));

        queue.push(order1);
        queue.push(order2);

        let prices: Vec<u64> = queue.iter().map(|order| order.price()).collect();
        assert_eq!(prices, vec![100, 101]);
    }

    #[test]
    fn test_order_queue_pop_after_remove() {
        let mut queue = OrderQueue::new();
        let order1 = Rc::new(create_test_order(1, 100, 10));
        let order2 = Rc::new(create_test_order(2, 101, 20));
        let order1_id = order1.id();

        queue.push(order1);
        queue.push(order2);

        // Remove the first order
        queue.remove(&order1_id);

        // Pop should return the second order
        let popped = queue.pop().unwrap();
        assert_eq!(popped.price(), 101);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_order_queue_multiple_operations() {
        let mut queue = OrderQueue::new();

        // Add several orders
        for i in 1..=5 {
            let order = Rc::new(create_test_order(i, 100 + i, 10 * i));
            queue.push(order);
        }

        assert_eq!(queue.len(), 5);

        // Remove order with id 3
        let order_id_3 = OrderId::from_u64(3);
        let removed = queue.remove(&order_id_3);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().price(), 103);
        assert_eq!(queue.len(), 4);

        // Pop two orders (should get orders 1 and 2)
        let order1 = queue.pop().unwrap();
        let order2 = queue.pop().unwrap();
        assert_eq!(order1.price(), 101);
        assert_eq!(order2.price(), 102);
        assert_eq!(queue.len(), 2);

        // Remaining orders should be 4 and 5
        let remaining = queue.to_vec();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].price(), 104);
        assert_eq!(remaining[1].price(), 105);
    }
}
