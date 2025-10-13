use crate::errors::PriceLevelError;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Tracks performance statistics for a price level
#[derive(Debug)]
pub struct PriceLevelStatistics {
    /// Number of orders added
    pub orders_added: AtomicUsize,

    /// Number of orders removed
    pub orders_removed: AtomicUsize,

    /// Number of orders executed
    pub orders_executed: AtomicUsize,

    /// Total quantity executed
    pub quantity_executed: AtomicU64,

    /// Total value executed
    pub value_executed: AtomicU64,

    /// Last execution timestamp
    pub last_execution_time: AtomicU64,

    /// First order arrival timestamp
    pub first_arrival_time: AtomicU64,

    /// Sum of waiting times for orders
    pub sum_waiting_time: AtomicU64,
}

impl PriceLevelStatistics {
    /// Create new empty statistics
    pub fn new() -> Self {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            orders_added: AtomicUsize::new(0),
            orders_removed: AtomicUsize::new(0),
            orders_executed: AtomicUsize::new(0),
            quantity_executed: AtomicU64::new(0),
            value_executed: AtomicU64::new(0),
            last_execution_time: AtomicU64::new(0),
            first_arrival_time: AtomicU64::new(current_time),
            sum_waiting_time: AtomicU64::new(0),
        }
    }

    /// Record a new order being added
    pub fn record_order_added(&self) {
        self.orders_added.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an order being removed without execution
    pub fn record_order_removed(&self) {
        self.orders_removed.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an order execution
    pub fn record_execution(&self, quantity: u64, price: u64, order_timestamp: u64) {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        self.orders_executed.fetch_add(1, Ordering::Relaxed);
        self.quantity_executed
            .fetch_add(quantity, Ordering::Relaxed);
        self.value_executed
            .fetch_add(quantity * price, Ordering::Relaxed);
        self.last_execution_time
            .store(current_time, Ordering::Relaxed);

        // Calculate waiting time for this order
        if order_timestamp > 0 {
            let waiting_time = current_time.saturating_sub(order_timestamp);
            self.sum_waiting_time
                .fetch_add(waiting_time, Ordering::Relaxed);
        }
    }

    /// Get total number of orders added
    pub fn orders_added(&self) -> usize {
        self.orders_added.load(Ordering::Relaxed)
    }

    /// Get total number of orders removed
    pub fn orders_removed(&self) -> usize {
        self.orders_removed.load(Ordering::Relaxed)
    }

    /// Get total number of orders executed
    pub fn orders_executed(&self) -> usize {
        self.orders_executed.load(Ordering::Relaxed)
    }

    /// Get total quantity executed
    pub fn quantity_executed(&self) -> u64 {
        self.quantity_executed.load(Ordering::Relaxed)
    }

    /// Get total value executed
    pub fn value_executed(&self) -> u64 {
        self.value_executed.load(Ordering::Relaxed)
    }

    /// Get average execution price
    pub fn average_execution_price(&self) -> Option<f64> {
        let qty = self.quantity_executed.load(Ordering::Relaxed);
        let value = self.value_executed.load(Ordering::Relaxed);

        if qty == 0 {
            None
        } else {
            Some(value as f64 / qty as f64)
        }
    }

    /// Get average waiting time for executed orders (in milliseconds)
    pub fn average_waiting_time(&self) -> Option<f64> {
        let count = self.orders_executed.load(Ordering::Relaxed);
        let sum = self.sum_waiting_time.load(Ordering::Relaxed);

        if count == 0 {
            None
        } else {
            Some(sum as f64 / count as f64)
        }
    }

    /// Get time since last execution (in milliseconds)
    pub fn time_since_last_execution(&self) -> Option<u64> {
        let last = self.last_execution_time.load(Ordering::Relaxed);
        if last == 0 {
            None
        } else {
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards")
                .as_millis() as u64;

            Some(current_time.saturating_sub(last))
        }
    }

    /// Reset all statistics
    pub fn reset(&self) {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_millis() as u64;

        self.orders_added.store(0, Ordering::Relaxed);
        self.orders_removed.store(0, Ordering::Relaxed);
        self.orders_executed.store(0, Ordering::Relaxed);
        self.quantity_executed.store(0, Ordering::Relaxed);
        self.value_executed.store(0, Ordering::Relaxed);
        self.last_execution_time.store(0, Ordering::Relaxed);
        self.first_arrival_time
            .store(current_time, Ordering::Relaxed);
        self.sum_waiting_time.store(0, Ordering::Relaxed);
    }
}

impl Default for PriceLevelStatistics {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PriceLevelStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PriceLevelStatistics:orders_added={};orders_removed={};orders_executed={};quantity_executed={};value_executed={};last_execution_time={};first_arrival_time={};sum_waiting_time={}",
            self.orders_added.load(Ordering::Relaxed),
            self.orders_removed.load(Ordering::Relaxed),
            self.orders_executed.load(Ordering::Relaxed),
            self.quantity_executed.load(Ordering::Relaxed),
            self.value_executed.load(Ordering::Relaxed),
            self.last_execution_time.load(Ordering::Relaxed),
            self.first_arrival_time.load(Ordering::Relaxed),
            self.sum_waiting_time.load(Ordering::Relaxed)
        )
    }
}

impl FromStr for PriceLevelStatistics {
    type Err = PriceLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 || parts[0] != "PriceLevelStatistics" {
            return Err(PriceLevelError::InvalidFormat);
        }

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

        let parse_usize = |field: &str, value: &str| -> Result<usize, PriceLevelError> {
            value
                .parse::<usize>()
                .map_err(|_| PriceLevelError::InvalidFieldValue {
                    field: field.to_string(),
                    value: value.to_string(),
                })
        };

        let parse_u64 = |field: &str, value: &str| -> Result<u64, PriceLevelError> {
            value
                .parse::<u64>()
                .map_err(|_| PriceLevelError::InvalidFieldValue {
                    field: field.to_string(),
                    value: value.to_string(),
                })
        };

        // Parse all fields
        let orders_added_str = get_field("orders_added")?;
        let orders_added = parse_usize("orders_added", orders_added_str)?;

        let orders_removed_str = get_field("orders_removed")?;
        let orders_removed = parse_usize("orders_removed", orders_removed_str)?;

        let orders_executed_str = get_field("orders_executed")?;
        let orders_executed = parse_usize("orders_executed", orders_executed_str)?;

        let quantity_executed_str = get_field("quantity_executed")?;
        let quantity_executed = parse_u64("quantity_executed", quantity_executed_str)?;

        let value_executed_str = get_field("value_executed")?;
        let value_executed = parse_u64("value_executed", value_executed_str)?;

        let last_execution_time_str = get_field("last_execution_time")?;
        let last_execution_time = parse_u64("last_execution_time", last_execution_time_str)?;

        let first_arrival_time_str = get_field("first_arrival_time")?;
        let first_arrival_time = parse_u64("first_arrival_time", first_arrival_time_str)?;

        let sum_waiting_time_str = get_field("sum_waiting_time")?;
        let sum_waiting_time = parse_u64("sum_waiting_time", sum_waiting_time_str)?;

        Ok(PriceLevelStatistics {
            orders_added: AtomicUsize::new(orders_added),
            orders_removed: AtomicUsize::new(orders_removed),
            orders_executed: AtomicUsize::new(orders_executed),
            quantity_executed: AtomicU64::new(quantity_executed),
            value_executed: AtomicU64::new(value_executed),
            last_execution_time: AtomicU64::new(last_execution_time),
            first_arrival_time: AtomicU64::new(first_arrival_time),
            sum_waiting_time: AtomicU64::new(sum_waiting_time),
        })
    }
}

impl Serialize for PriceLevelStatistics {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("PriceLevelStatistics", 8)?;

        state.serialize_field("orders_added", &self.orders_added.load(Ordering::Relaxed))?;
        state.serialize_field(
            "orders_removed",
            &self.orders_removed.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "orders_executed",
            &self.orders_executed.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "quantity_executed",
            &self.quantity_executed.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "value_executed",
            &self.value_executed.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "last_execution_time",
            &self.last_execution_time.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "first_arrival_time",
            &self.first_arrival_time.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "sum_waiting_time",
            &self.sum_waiting_time.load(Ordering::Relaxed),
        )?;

        state.end()
    }
}

impl<'de> Deserialize<'de> for PriceLevelStatistics {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            OrdersAdded,
            OrdersRemoved,
            OrdersExecuted,
            QuantityExecuted,
            ValueExecuted,
            LastExecutionTime,
            FirstArrivalTime,
            SumWaitingTime,
        }

        impl<'de> Deserialize<'de> for Field {
            fn deserialize<D>(deserializer: D) -> Result<Field, D::Error>
            where
                D: Deserializer<'de>,
            {
                struct FieldVisitor;

                impl Visitor<'_> for FieldVisitor {
                    type Value = Field;

                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("field name")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "orders_added" => Ok(Field::OrdersAdded),
                            "orders_removed" => Ok(Field::OrdersRemoved),
                            "orders_executed" => Ok(Field::OrdersExecuted),
                            "quantity_executed" => Ok(Field::QuantityExecuted),
                            "value_executed" => Ok(Field::ValueExecuted),
                            "last_execution_time" => Ok(Field::LastExecutionTime),
                            "first_arrival_time" => Ok(Field::FirstArrivalTime),
                            "sum_waiting_time" => Ok(Field::SumWaitingTime),
                            _ => Err(de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct StatisticsVisitor;

        impl<'de> Visitor<'de> for StatisticsVisitor {
            type Value = PriceLevelStatistics;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct PriceLevelStatistics")
            }

            fn visit_map<V>(self, mut map: V) -> Result<PriceLevelStatistics, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut orders_added = None;
                let mut orders_removed = None;
                let mut orders_executed = None;
                let mut quantity_executed = None;
                let mut value_executed = None;
                let mut last_execution_time = None;
                let mut first_arrival_time = None;
                let mut sum_waiting_time = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::OrdersAdded => {
                            if orders_added.is_some() {
                                return Err(de::Error::duplicate_field("orders_added"));
                            }
                            orders_added = Some(map.next_value()?);
                        }
                        Field::OrdersRemoved => {
                            if orders_removed.is_some() {
                                return Err(de::Error::duplicate_field("orders_removed"));
                            }
                            orders_removed = Some(map.next_value()?);
                        }
                        Field::OrdersExecuted => {
                            if orders_executed.is_some() {
                                return Err(de::Error::duplicate_field("orders_executed"));
                            }
                            orders_executed = Some(map.next_value()?);
                        }
                        Field::QuantityExecuted => {
                            if quantity_executed.is_some() {
                                return Err(de::Error::duplicate_field("quantity_executed"));
                            }
                            quantity_executed = Some(map.next_value()?);
                        }
                        Field::ValueExecuted => {
                            if value_executed.is_some() {
                                return Err(de::Error::duplicate_field("value_executed"));
                            }
                            value_executed = Some(map.next_value()?);
                        }
                        Field::LastExecutionTime => {
                            if last_execution_time.is_some() {
                                return Err(de::Error::duplicate_field("last_execution_time"));
                            }
                            last_execution_time = Some(map.next_value()?);
                        }
                        Field::FirstArrivalTime => {
                            if first_arrival_time.is_some() {
                                return Err(de::Error::duplicate_field("first_arrival_time"));
                            }
                            first_arrival_time = Some(map.next_value()?);
                        }
                        Field::SumWaitingTime => {
                            if sum_waiting_time.is_some() {
                                return Err(de::Error::duplicate_field("sum_waiting_time"));
                            }
                            sum_waiting_time = Some(map.next_value()?);
                        }
                    }
                }

                let orders_added = orders_added.unwrap_or(0);
                let orders_removed = orders_removed.unwrap_or(0);
                let orders_executed = orders_executed.unwrap_or(0);
                let quantity_executed = quantity_executed.unwrap_or(0);
                let value_executed = value_executed.unwrap_or(0);
                let last_execution_time = last_execution_time.unwrap_or(0);

                let first_arrival_time = first_arrival_time.unwrap_or_else(|| {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64
                });

                let sum_waiting_time = sum_waiting_time.unwrap_or(0);

                Ok(PriceLevelStatistics {
                    orders_added: AtomicUsize::new(orders_added),
                    orders_removed: AtomicUsize::new(orders_removed),
                    orders_executed: AtomicUsize::new(orders_executed),
                    quantity_executed: AtomicU64::new(quantity_executed),
                    value_executed: AtomicU64::new(value_executed),
                    last_execution_time: AtomicU64::new(last_execution_time),
                    first_arrival_time: AtomicU64::new(first_arrival_time),
                    sum_waiting_time: AtomicU64::new(sum_waiting_time),
                })
            }
        }

        const FIELDS: &[&str] = &[
            "orders_added",
            "orders_removed",
            "orders_executed",
            "quantity_executed",
            "value_executed",
            "last_execution_time",
            "first_arrival_time",
            "sum_waiting_time",
        ];

        deserializer.deserialize_struct("PriceLevelStatistics", FIELDS, StatisticsVisitor)
    }
}

#[cfg(test)]
mod tests {
    use crate::price_level::PriceLevelStatistics;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn test_new() {
        let stats = PriceLevelStatistics::new();
        assert_eq!(stats.orders_added(), 0);
        assert_eq!(stats.orders_removed(), 0);
        assert_eq!(stats.orders_executed(), 0);
        assert_eq!(stats.quantity_executed(), 0);
        assert_eq!(stats.value_executed(), 0);
        assert_eq!(stats.last_execution_time.load(Ordering::Relaxed), 0);
        assert!(stats.first_arrival_time.load(Ordering::Relaxed) > 0);
        assert_eq!(stats.sum_waiting_time.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_default() {
        let stats = PriceLevelStatistics::default();
        assert_eq!(stats.orders_added(), 0);
        assert_eq!(stats.orders_removed(), 0);
        assert_eq!(stats.orders_executed(), 0);
    }

    #[test]
    fn test_record_operations() {
        let stats = PriceLevelStatistics::new();

        // Test recording added orders
        for _ in 0..5 {
            stats.record_order_added();
        }
        assert_eq!(stats.orders_added(), 5);

        // Test recording removed orders
        for _ in 0..3 {
            stats.record_order_removed();
        }
        assert_eq!(stats.orders_removed(), 3);

        // Test recording executed orders
        stats.record_execution(10, 100, 0); // qty=10, price=100, no timestamp
        assert_eq!(stats.orders_executed(), 1);
        assert_eq!(stats.quantity_executed(), 10);
        assert_eq!(stats.value_executed(), 1000); // 10 * 100
        assert!(stats.last_execution_time.load(Ordering::Relaxed) > 0);

        // Test with timestamp
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - 1000; // 1 second ago

        // Sleep to ensure waiting time is measurable
        thread::sleep(Duration::from_millis(10));

        stats.record_execution(5, 200, timestamp);
        assert_eq!(stats.orders_executed(), 2);
        assert_eq!(stats.quantity_executed(), 15); // 10 + 5
        assert_eq!(stats.value_executed(), 2000); // 1000 + (5 * 200)
        assert!(stats.sum_waiting_time.load(Ordering::Relaxed) >= 1000); // At least 1 second waiting time
    }

    #[test]
    fn test_average_execution_price() {
        let stats = PriceLevelStatistics::new();

        // Test with no executions
        assert_eq!(stats.average_execution_price(), None);

        // Test with executions
        stats.record_execution(10, 100, 0); // Total value: 1000
        stats.record_execution(20, 150, 0); // Total value: 3000 + 1000 = 4000

        // Average price should be 4000 / 30 = 133.33...
        let avg_price = stats.average_execution_price().unwrap();
        assert!((avg_price - 133.33).abs() < 0.01);
    }

    #[test]
    fn test_average_waiting_time() {
        let stats = PriceLevelStatistics::new();

        // Test with no executions
        assert_eq!(stats.average_waiting_time(), None);

        // Test with executions
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        stats.record_execution(10, 100, now - 1000); // 1 second ago
        stats.record_execution(20, 150, now - 3000); // 3 seconds ago

        // Total waiting time: 1000 + 3000 = 4000ms, average = 2000ms
        let avg_wait = stats.average_waiting_time().unwrap();
        assert!((1900.0..=2100.0).contains(&avg_wait));
    }

    #[test]
    fn test_time_since_last_execution() {
        let stats = PriceLevelStatistics::new();

        // Test with no executions
        assert_eq!(stats.time_since_last_execution(), None);

        // Record an execution
        stats.record_execution(10, 100, 0);

        // Sleep a bit to ensure time passes
        thread::sleep(Duration::from_millis(10));

        // Should return some non-zero value
        let time_since = stats.time_since_last_execution().unwrap();
        assert!(time_since > 0);
    }

    #[test]
    fn test_reset() {
        let stats = PriceLevelStatistics::new();

        // Add some data
        stats.record_order_added();
        stats.record_order_removed();
        stats.record_execution(10, 100, 0);

        // Verify data was recorded
        assert_eq!(stats.orders_added(), 1);
        assert_eq!(stats.orders_removed(), 1);
        assert_eq!(stats.orders_executed(), 1);

        // Reset stats
        stats.reset();

        // Verify reset worked
        assert_eq!(stats.orders_added(), 0);
        assert_eq!(stats.orders_removed(), 0);
        assert_eq!(stats.orders_executed(), 0);
        assert_eq!(stats.quantity_executed(), 0);
        assert_eq!(stats.value_executed(), 0);
        assert_eq!(stats.last_execution_time.load(Ordering::Relaxed), 0);
        assert!(stats.first_arrival_time.load(Ordering::Relaxed) > 0);
        assert_eq!(stats.sum_waiting_time.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_display() {
        let stats = PriceLevelStatistics::new();

        // Add some data
        stats.record_order_added();
        stats.record_order_removed();
        stats.record_execution(10, 100, 0);

        // Get display string
        let display_str = stats.to_string();

        // Verify format
        assert!(display_str.starts_with("PriceLevelStatistics:"));
        assert!(display_str.contains("orders_added=1"));
        assert!(display_str.contains("orders_removed=1"));
        assert!(display_str.contains("orders_executed=1"));
        assert!(display_str.contains("quantity_executed=10"));
        assert!(display_str.contains("value_executed=1000"));
    }

    #[test]
    fn test_from_str() {
        // Create sample string representation
        let input = "PriceLevelStatistics:orders_added=5;orders_removed=3;orders_executed=2;quantity_executed=15;value_executed=2000;last_execution_time=1616823000000;first_arrival_time=1616823000001;sum_waiting_time=1000";

        // Parse from string
        let stats = PriceLevelStatistics::from_str(input).unwrap();

        // Verify values
        assert_eq!(stats.orders_added(), 5);
        assert_eq!(stats.orders_removed(), 3);
        assert_eq!(stats.orders_executed(), 2);
        assert_eq!(stats.quantity_executed(), 15);
        assert_eq!(stats.value_executed(), 2000);
        assert_eq!(
            stats.last_execution_time.load(Ordering::Relaxed),
            1616823000000
        );
        assert_eq!(
            stats.first_arrival_time.load(Ordering::Relaxed),
            1616823000001
        );
        assert_eq!(stats.sum_waiting_time.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn test_from_str_invalid_format() {
        let input = "InvalidFormat";
        assert!(PriceLevelStatistics::from_str(input).is_err());
    }

    #[test]
    fn test_from_str_missing_field() {
        // Missing sum_waiting_time
        let input = "PriceLevelStatistics:orders_added=5;orders_removed=3;orders_executed=2;quantity_executed=15;value_executed=2000;last_execution_time=1616823000000;first_arrival_time=1616823000001";
        assert!(PriceLevelStatistics::from_str(input).is_err());
    }

    #[test]
    fn test_from_str_invalid_field_value() {
        // Invalid orders_added (not a number)
        let input = "PriceLevelStatistics:orders_added=invalid;orders_removed=3;orders_executed=2;quantity_executed=15;value_executed=2000;last_execution_time=1616823000000;first_arrival_time=1616823000001;sum_waiting_time=1000";
        assert!(PriceLevelStatistics::from_str(input).is_err());
    }

    #[test]
    fn test_serialize_deserialize_json() {
        let stats = PriceLevelStatistics::new();

        // Add some data
        stats.record_order_added();
        stats.record_order_removed();
        stats.record_execution(10, 100, 0);

        // Serialize to JSON
        let json = serde_json::to_string(&stats).unwrap();

        // Verify JSON format
        assert!(json.contains("\"orders_added\":1"));
        assert!(json.contains("\"orders_removed\":1"));
        assert!(json.contains("\"orders_executed\":1"));
        assert!(json.contains("\"quantity_executed\":10"));
        assert!(json.contains("\"value_executed\":1000"));

        // Deserialize from JSON
        let deserialized: PriceLevelStatistics = serde_json::from_str(&json).unwrap();

        // Verify values
        assert_eq!(deserialized.orders_added(), 1);
        assert_eq!(deserialized.orders_removed(), 1);
        assert_eq!(deserialized.orders_executed(), 1);
        assert_eq!(deserialized.quantity_executed(), 10);
        assert_eq!(deserialized.value_executed(), 1000);
    }

    #[test]
    fn test_round_trip_display_parse() {
        let stats = PriceLevelStatistics::new();

        // Use precise timestamps to avoid timing issues
        let current_time: u64 = 1616823000000;
        stats
            .last_execution_time
            .store(current_time, Ordering::Relaxed);
        stats
            .first_arrival_time
            .store(current_time + 1, Ordering::Relaxed);

        // Add some data
        stats.record_order_added();
        stats.record_order_added();
        stats.record_order_removed();

        // Manual record to have predictable values
        stats.orders_executed.store(2, Ordering::Relaxed);
        stats.quantity_executed.store(15, Ordering::Relaxed);
        stats.value_executed.store(2000, Ordering::Relaxed);
        stats.sum_waiting_time.store(1000, Ordering::Relaxed);

        // Convert to string
        let string_representation = stats.to_string();

        // Parse back
        let parsed = PriceLevelStatistics::from_str(&string_representation).unwrap();

        // Verify values match
        assert_eq!(parsed.orders_added(), stats.orders_added());
        assert_eq!(parsed.orders_removed(), stats.orders_removed());
        assert_eq!(parsed.orders_executed(), stats.orders_executed());
        assert_eq!(parsed.quantity_executed(), stats.quantity_executed());
        assert_eq!(parsed.value_executed(), stats.value_executed());
        assert_eq!(
            parsed.last_execution_time.load(Ordering::Relaxed),
            stats.last_execution_time.load(Ordering::Relaxed)
        );
        assert_eq!(
            parsed.first_arrival_time.load(Ordering::Relaxed),
            stats.first_arrival_time.load(Ordering::Relaxed)
        );
        assert_eq!(
            parsed.sum_waiting_time.load(Ordering::Relaxed),
            stats.sum_waiting_time.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn test_thread_safety() {
        let stats = PriceLevelStatistics::new();
        let stats_arc = Arc::new(stats);

        let mut handles = vec![];

        // Spawn 10 threads to concurrently update stats
        for _ in 0..10 {
            let stats_clone = Arc::clone(&stats_arc);
            let handle = thread::spawn(move || {
                for _ in 0..100 {
                    stats_clone.record_order_added();
                    stats_clone.record_order_removed();
                    stats_clone.record_execution(1, 100, 0);
                }
            });
            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify final counts
        assert_eq!(stats_arc.orders_added(), 1000); // 10 threads * 100 calls
        assert_eq!(stats_arc.orders_removed(), 1000);
        assert_eq!(stats_arc.orders_executed(), 1000);
        assert_eq!(stats_arc.quantity_executed(), 1000);
        assert_eq!(stats_arc.value_executed(), 100000); // 1000 * 100
    }

    #[test]
    fn test_statistics_reset_and_verify() {
        let stats = PriceLevelStatistics::new();

        // Add some data
        stats.record_order_added();
        stats.record_order_added();
        stats.record_order_removed();
        stats.record_execution(10, 100, 0);

        // Verify stats were recorded
        assert_eq!(stats.orders_added(), 2);
        assert_eq!(stats.orders_removed(), 1);
        assert_eq!(stats.orders_executed(), 1);

        // Reset stats
        stats.reset();

        // Verify all statistics are reset
        assert_eq!(stats.orders_added(), 0);
        assert_eq!(stats.orders_removed(), 0);
        assert_eq!(stats.orders_executed(), 0);
        assert_eq!(stats.quantity_executed(), 0);
        assert_eq!(stats.value_executed(), 0);
        assert_eq!(stats.last_execution_time.load(Ordering::Relaxed), 0);
        assert!(stats.first_arrival_time.load(Ordering::Relaxed) > 0);
        assert_eq!(stats.sum_waiting_time.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_statistics_serialize_deserialize_fields() {
        let stats = PriceLevelStatistics::new();

        // Set and verify each field
        stats.orders_added.store(1, Ordering::Relaxed);
        stats.orders_removed.store(2, Ordering::Relaxed);
        stats.orders_executed.store(3, Ordering::Relaxed);
        stats.quantity_executed.store(4, Ordering::Relaxed);
        stats.value_executed.store(5, Ordering::Relaxed);
        stats.last_execution_time.store(6, Ordering::Relaxed);
        stats.first_arrival_time.store(7, Ordering::Relaxed);
        stats.sum_waiting_time.store(8, Ordering::Relaxed);

        // Serialize to JSON
        let serialized = serde_json::to_string(&stats).unwrap();

        // Should contain all the field values
        assert!(serialized.contains("\"orders_added\":1"));
        assert!(serialized.contains("\"orders_removed\":2"));
        assert!(serialized.contains("\"orders_executed\":3"));
        assert!(serialized.contains("\"quantity_executed\":4"));
        assert!(serialized.contains("\"value_executed\":5"));
        assert!(serialized.contains("\"last_execution_time\":6"));
        assert!(serialized.contains("\"first_arrival_time\":7"));
        assert!(serialized.contains("\"sum_waiting_time\":8"));

        // Deserialize back
        let deserialized: PriceLevelStatistics = serde_json::from_str(&serialized).unwrap();

        // Verify all fields are deserialized correctly
        assert_eq!(deserialized.orders_added(), 1);
        assert_eq!(deserialized.orders_removed(), 2);
        assert_eq!(deserialized.orders_executed(), 3);
        assert_eq!(deserialized.quantity_executed(), 4);
        assert_eq!(deserialized.value_executed(), 5);
        assert_eq!(deserialized.last_execution_time.load(Ordering::Relaxed), 6);
        assert_eq!(deserialized.first_arrival_time.load(Ordering::Relaxed), 7);
        assert_eq!(deserialized.sum_waiting_time.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn test_statistics_visitor_missing_fields() {
        // Test with a partial JSON
        let json = r#"{
        "orders_added": 1,
        "orders_removed": 2,
        "orders_executed": 3
    }"#;

        // Should still deserialize correctly with default values for missing fields
        let deserialized: PriceLevelStatistics = serde_json::from_str(json).unwrap();

        assert_eq!(deserialized.orders_added(), 1);
        assert_eq!(deserialized.orders_removed(), 2);
        assert_eq!(deserialized.orders_executed(), 3);
        assert_eq!(deserialized.quantity_executed(), 0);
        assert_eq!(deserialized.value_executed(), 0);
    }
}
