use crate::errors::PriceLevelError;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Tracks performance statistics for a price level
#[derive(Debug)]
pub struct PriceLevelStatistics {
    /// Number of orders added
    pub orders_added: usize,

    /// Number of orders removed
    pub orders_removed: usize,

    /// Number of orders executed
    pub orders_executed: usize,

    /// Total quantity executed
    pub quantity_executed: u64,

    /// Total value executed
    pub value_executed: u64,

    /// Last execution timestamp
    pub last_execution_time: u64,

    /// First order arrival timestamp
    pub first_arrival_time: u64,

    /// Sum of waiting times for orders
    pub sum_waiting_time: u64,
}

impl PriceLevelStatistics {
    /// Create new empty statistics
    pub fn new() -> Self {
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            orders_added: 0,
            orders_removed: 0,
            orders_executed: 0,
            quantity_executed: 0,
            value_executed: 0,
            last_execution_time: 0,
            first_arrival_time: current_time,
            sum_waiting_time: 0,
        }
    }

    /// Record an order being added
    pub fn record_order_added(&mut self) {
        self.orders_added += 1;
    }

    /// Record an order being removed
    pub fn record_order_removed(&mut self) {
        self.orders_removed += 1;
    }

    /// Record an execution
    pub fn record_execution(&mut self, quantity: u64, price: u64, waiting_time: u64) {
        self.orders_executed += 1;
        self.quantity_executed += quantity;
        self.value_executed += quantity * price;
        self.sum_waiting_time += waiting_time;
        self.last_execution_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
    }

    /// Get the number of orders added
    pub fn orders_added(&self) -> usize {
        self.orders_added
    }

    /// Get the number of orders removed
    pub fn orders_removed(&self) -> usize {
        self.orders_removed
    }

    /// Get the number of orders executed
    pub fn orders_executed(&self) -> usize {
        self.orders_executed
    }

    /// Get the total quantity executed
    pub fn quantity_executed(&self) -> u64 {
        self.quantity_executed
    }

    /// Get the total value executed
    pub fn value_executed(&self) -> u64 {
        self.value_executed
    }

    /// Get the average execution price
    pub fn average_execution_price(&self) -> f64 {
        if self.quantity_executed > 0 {
            self.value_executed as f64 / self.quantity_executed as f64
        } else {
            0.0
        }
    }

    /// Get the average waiting time
    pub fn average_waiting_time(&self) -> f64 {
        if self.orders_executed > 0 {
            self.sum_waiting_time as f64 / self.orders_executed as f64
        } else {
            0.0
        }
    }

    /// Get the time since last execution in milliseconds
    pub fn time_since_last_execution(&self) -> u64 {
        if self.last_execution_time > 0 {
            (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64)
                .saturating_sub(self.last_execution_time)
        } else {
            0
        }
    }

    /// Reset all statistics
    pub fn reset(&mut self) {
        self.orders_added = 0;
        self.orders_removed = 0;
        self.orders_executed = 0;
        self.quantity_executed = 0;
        self.value_executed = 0;
        self.last_execution_time = 0;
        self.first_arrival_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.sum_waiting_time = 0;
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
            "orders_added:{},orders_removed:{},orders_executed:{},quantity_executed:{},value_executed:{},last_execution_time:{},first_arrival_time:{},sum_waiting_time:{}",
            self.orders_added,
            self.orders_removed,
            self.orders_executed,
            self.quantity_executed,
            self.value_executed,
            self.last_execution_time,
            self.first_arrival_time,
            self.sum_waiting_time
        )
    }
}

impl FromStr for PriceLevelStatistics {
    type Err = PriceLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut orders_added = 0;
        let mut orders_removed = 0;
        let mut orders_executed = 0;
        let mut quantity_executed = 0;
        let mut value_executed = 0;
        let mut last_execution_time = 0;
        let mut first_arrival_time = 0;
        let mut sum_waiting_time = 0;

        for pair in s.split(',') {
            let parts: Vec<&str> = pair.split(':').collect();
            if parts.len() != 2 {
                return Err(PriceLevelError::InvalidFormat(format!(
                    "Invalid key-value pair: {}",
                    pair
                )));
            }

            let key = parts[0].trim();
            let value = parts[1].trim();

            match key {
                "orders_added" => {
                    orders_added = value.parse().map_err(|_| {
                        PriceLevelError::InvalidFormat(format!("Invalid orders_added: {}", value))
                    })?
                }
                "orders_removed" => {
                    orders_removed = value.parse().map_err(|_| {
                        PriceLevelError::InvalidFormat(format!("Invalid orders_removed: {}", value))
                    })?
                }
                "orders_executed" => {
                    orders_executed = value.parse().map_err(|_| {
                        PriceLevelError::InvalidFormat(format!(
                            "Invalid orders_executed: {}",
                            value
                        ))
                    })?
                }
                "quantity_executed" => {
                    quantity_executed = value.parse().map_err(|_| {
                        PriceLevelError::InvalidFormat(format!(
                            "Invalid quantity_executed: {}",
                            value
                        ))
                    })?
                }
                "value_executed" => {
                    value_executed = value.parse().map_err(|_| {
                        PriceLevelError::InvalidFormat(format!("Invalid value_executed: {}", value))
                    })?
                }
                "last_execution_time" => {
                    last_execution_time = value.parse().map_err(|_| {
                        PriceLevelError::InvalidFormat(format!(
                            "Invalid last_execution_time: {}",
                            value
                        ))
                    })?
                }
                "first_arrival_time" => {
                    first_arrival_time = value.parse().map_err(|_| {
                        PriceLevelError::InvalidFormat(format!(
                            "Invalid first_arrival_time: {}",
                            value
                        ))
                    })?
                }
                "sum_waiting_time" => {
                    sum_waiting_time = value.parse().map_err(|_| {
                        PriceLevelError::InvalidFormat(format!(
                            "Invalid sum_waiting_time: {}",
                            value
                        ))
                    })?
                }
                _ => {
                    return Err(PriceLevelError::InvalidFormat(format!(
                        "Unknown key: {}",
                        key
                    )));
                }
            }
        }

        Ok(Self {
            orders_added,
            orders_removed,
            orders_executed,
            quantity_executed,
            value_executed,
            last_execution_time,
            first_arrival_time,
            sum_waiting_time,
        })
    }
}

impl Serialize for PriceLevelStatistics {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("PriceLevelStatistics", 8)?;
        state.serialize_field("orders_added", &self.orders_added)?;
        state.serialize_field("orders_removed", &self.orders_removed)?;
        state.serialize_field("orders_executed", &self.orders_executed)?;
        state.serialize_field("quantity_executed", &self.quantity_executed)?;
        state.serialize_field("value_executed", &self.value_executed)?;
        state.serialize_field("last_execution_time", &self.last_execution_time)?;
        state.serialize_field("first_arrival_time", &self.first_arrival_time)?;
        state.serialize_field("sum_waiting_time", &self.sum_waiting_time)?;
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

                impl<'de> Visitor<'de> for FieldVisitor {
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
                                return Err(serde::de::Error::duplicate_field("orders_added"));
                            }
                            orders_added = Some(map.next_value()?);
                        }
                        Field::OrdersRemoved => {
                            if orders_removed.is_some() {
                                return Err(serde::de::Error::duplicate_field("orders_removed"));
                            }
                            orders_removed = Some(map.next_value()?);
                        }
                        Field::OrdersExecuted => {
                            if orders_executed.is_some() {
                                return Err(serde::de::Error::duplicate_field("orders_executed"));
                            }
                            orders_executed = Some(map.next_value()?);
                        }
                        Field::QuantityExecuted => {
                            if quantity_executed.is_some() {
                                return Err(serde::de::Error::duplicate_field("quantity_executed"));
                            }
                            quantity_executed = Some(map.next_value()?);
                        }
                        Field::ValueExecuted => {
                            if value_executed.is_some() {
                                return Err(serde::de::Error::duplicate_field("value_executed"));
                            }
                            value_executed = Some(map.next_value()?);
                        }
                        Field::LastExecutionTime => {
                            if last_execution_time.is_some() {
                                return Err(serde::de::Error::duplicate_field(
                                    "last_execution_time",
                                ));
                            }
                            last_execution_time = Some(map.next_value()?);
                        }
                        Field::FirstArrivalTime => {
                            if first_arrival_time.is_some() {
                                return Err(serde::de::Error::duplicate_field(
                                    "first_arrival_time",
                                ));
                            }
                            first_arrival_time = Some(map.next_value()?);
                        }
                        Field::SumWaitingTime => {
                            if sum_waiting_time.is_some() {
                                return Err(serde::de::Error::duplicate_field("sum_waiting_time"));
                            }
                            sum_waiting_time = Some(map.next_value()?);
                        }
                    }
                }

                let orders_added =
                    orders_added.ok_or_else(|| serde::de::Error::missing_field("orders_added"))?;
                let orders_removed = orders_removed
                    .ok_or_else(|| serde::de::Error::missing_field("orders_removed"))?;
                let orders_executed = orders_executed
                    .ok_or_else(|| serde::de::Error::missing_field("orders_executed"))?;
                let quantity_executed = quantity_executed
                    .ok_or_else(|| serde::de::Error::missing_field("quantity_executed"))?;
                let value_executed = value_executed
                    .ok_or_else(|| serde::de::Error::missing_field("value_executed"))?;
                let last_execution_time = last_execution_time
                    .ok_or_else(|| serde::de::Error::missing_field("last_execution_time"))?;
                let first_arrival_time = first_arrival_time
                    .ok_or_else(|| serde::de::Error::missing_field("first_arrival_time"))?;
                let sum_waiting_time = sum_waiting_time
                    .ok_or_else(|| serde::de::Error::missing_field("sum_waiting_time"))?;

                Ok(PriceLevelStatistics {
                    orders_added,
                    orders_removed,
                    orders_executed,
                    quantity_executed,
                    value_executed,
                    last_execution_time,
                    first_arrival_time,
                    sum_waiting_time,
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
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_new() {
        let stats = PriceLevelStatistics::new();
        assert_eq!(stats.orders_added(), 0);
        assert_eq!(stats.orders_removed(), 0);
        assert_eq!(stats.orders_executed(), 0);
        assert_eq!(stats.quantity_executed(), 0);
        assert_eq!(stats.value_executed(), 0);
    }

    #[test]
    fn test_default() {
        let stats = PriceLevelStatistics::default();
        assert_eq!(stats.orders_added(), 0);
    }

    #[test]
    fn test_record_operations() {
        let mut stats = PriceLevelStatistics::new();

        stats.record_order_added();
        assert_eq!(stats.orders_added(), 1);

        stats.record_order_removed();
        assert_eq!(stats.orders_removed(), 1);

        stats.record_execution(100, 50, 1000);
        assert_eq!(stats.orders_executed(), 1);
        assert_eq!(stats.quantity_executed(), 100);
        assert_eq!(stats.value_executed(), 5000);

        stats.record_execution(50, 60, 2000);
        assert_eq!(stats.orders_executed(), 2);
        assert_eq!(stats.quantity_executed(), 150);
        assert_eq!(stats.value_executed(), 8000);
    }

    #[test]
    fn test_average_execution_price() {
        let mut stats = PriceLevelStatistics::new();

        assert_eq!(stats.average_execution_price(), 0.0);

        stats.record_execution(100, 50, 1000);
        assert_eq!(stats.average_execution_price(), 50.0);

        stats.record_execution(50, 60, 2000);
        assert_eq!(stats.average_execution_price(), 8000.0 / 150.0);
    }

    #[test]
    fn test_average_waiting_time() {
        let mut stats = PriceLevelStatistics::new();

        assert_eq!(stats.average_waiting_time(), 0.0);

        stats.record_execution(100, 50, 1000);
        assert_eq!(stats.average_waiting_time(), 1000.0);

        stats.record_execution(50, 60, 2000);
        assert_eq!(stats.average_waiting_time(), 1500.0);
    }

    #[test]
    fn test_time_since_last_execution() {
        let mut stats = PriceLevelStatistics::new();

        assert_eq!(stats.time_since_last_execution(), 0);

        stats.record_execution(100, 50, 1000);
        thread::sleep(Duration::from_millis(10));

        let time_since = stats.time_since_last_execution();
        assert!(time_since >= 10);
    }

    #[test]
    fn test_reset() {
        let mut stats = PriceLevelStatistics::new();

        stats.record_order_added();
        stats.record_order_removed();
        stats.record_execution(100, 50, 1000);

        stats.reset();

        assert_eq!(stats.orders_added(), 0);
        assert_eq!(stats.orders_removed(), 0);
        assert_eq!(stats.orders_executed(), 0);
        assert_eq!(stats.quantity_executed(), 0);
        assert_eq!(stats.value_executed(), 0);
    }

    #[test]
    fn test_display() {
        let mut stats = PriceLevelStatistics::new();
        stats.record_order_added();
        stats.record_execution(100, 50, 1000);

        let display_str = format!("{}", stats);
        assert!(display_str.contains("orders_added:1"));
        assert!(display_str.contains("orders_executed:1"));
        assert!(display_str.contains("quantity_executed:100"));
        assert!(display_str.contains("value_executed:5000"));
    }

    #[test]
    fn test_from_str() {
        let stats_str = "orders_added:1,orders_removed:2,orders_executed:3,quantity_executed:400,value_executed:5000,last_execution_time:600,first_arrival_time:700,sum_waiting_time:800";
        let stats = PriceLevelStatistics::from_str(stats_str).unwrap();

        assert_eq!(stats.orders_added(), 1);
        assert_eq!(stats.orders_removed(), 2);
        assert_eq!(stats.orders_executed(), 3);
        assert_eq!(stats.quantity_executed(), 400);
        assert_eq!(stats.value_executed(), 5000);
        assert_eq!(stats.last_execution_time, 600);
        assert_eq!(stats.first_arrival_time, 700);
        assert_eq!(stats.sum_waiting_time, 800);
    }

    #[test]
    fn test_from_str_invalid_format() {
        let result = PriceLevelStatistics::from_str("invalid_format");
        assert!(result.is_err());
    }

    #[test]
    fn test_from_str_missing_field() {
        let result = PriceLevelStatistics::from_str("orders_added:1");
        assert!(result.is_ok());
        let stats = result.unwrap();
        assert_eq!(stats.orders_added(), 1);
        assert_eq!(stats.orders_removed(), 0); // default value for missing field
    }

    #[test]
    fn test_from_str_invalid_field_value() {
        let result = PriceLevelStatistics::from_str(
            "orders_added:invalid,orders_removed:0,orders_executed:0,quantity_executed:0,value_executed:0,last_execution_time:0,first_arrival_time:0,sum_waiting_time:0",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_serialize_deserialize_json() {
        let mut original_stats = PriceLevelStatistics::new();
        original_stats.record_order_added();
        original_stats.record_execution(100, 50, 1000);

        let json_str = serde_json::to_string(&original_stats).unwrap();
        let deserialized_stats: PriceLevelStatistics = serde_json::from_str(&json_str).unwrap();

        assert_eq!(
            original_stats.orders_added(),
            deserialized_stats.orders_added()
        );
        assert_eq!(
            original_stats.orders_executed(),
            deserialized_stats.orders_executed()
        );
        assert_eq!(
            original_stats.quantity_executed(),
            deserialized_stats.quantity_executed()
        );
        assert_eq!(
            original_stats.value_executed(),
            deserialized_stats.value_executed()
        );
    }

    #[test]
    fn test_round_trip_display_parse() {
        let mut original_stats = PriceLevelStatistics::new();
        original_stats.record_order_added();
        original_stats.record_order_removed();
        original_stats.record_execution(150, 25, 2500);
        original_stats.record_execution(75, 30, 1200);

        let display_str = format!("{}", original_stats);
        let parsed_stats = PriceLevelStatistics::from_str(&display_str).unwrap();

        assert_eq!(original_stats.orders_added(), parsed_stats.orders_added());
        assert_eq!(
            original_stats.orders_removed(),
            parsed_stats.orders_removed()
        );
        assert_eq!(
            original_stats.orders_executed(),
            parsed_stats.orders_executed()
        );
        assert_eq!(
            original_stats.quantity_executed(),
            parsed_stats.quantity_executed()
        );
        assert_eq!(
            original_stats.value_executed(),
            parsed_stats.value_executed()
        );
        assert_eq!(
            original_stats.last_execution_time,
            parsed_stats.last_execution_time
        );
        assert_eq!(
            original_stats.first_arrival_time,
            parsed_stats.first_arrival_time
        );
        assert_eq!(
            original_stats.sum_waiting_time,
            parsed_stats.sum_waiting_time
        );

        let parsed_display_str = format!("{}", parsed_stats);
        assert_eq!(display_str, parsed_display_str);
    }

    #[test]
    fn test_thread_safety() {
        // Since this is now single-threaded, this test just ensures basic functionality
        let mut stats = PriceLevelStatistics::new();

        for i in 0..10 {
            stats.record_order_added();
            stats.record_execution(10, i + 1, 100 * (i + 1));
        }

        assert_eq!(stats.orders_added(), 10);
        assert_eq!(stats.orders_executed(), 10);
        assert_eq!(stats.quantity_executed(), 100);
    }

    #[test]
    fn test_statistics_reset_and_verify() {
        let mut stats = PriceLevelStatistics::new();

        for i in 0..5 {
            stats.record_order_added();
            stats.record_order_removed();
            stats.record_execution(20, 100 + i, 500);
        }

        assert!(stats.orders_added() > 0);
        assert!(stats.orders_removed() > 0);
        assert!(stats.orders_executed() > 0);
        assert!(stats.quantity_executed() > 0);
        assert!(stats.value_executed() > 0);

        stats.reset();

        assert_eq!(stats.orders_added(), 0);
        assert_eq!(stats.orders_removed(), 0);
        assert_eq!(stats.orders_executed(), 0);
        assert_eq!(stats.quantity_executed(), 0);
        assert_eq!(stats.value_executed(), 0);
    }

    #[test]
    fn test_statistics_serialize_deserialize_fields() {
        let mut stats = PriceLevelStatistics::new();
        stats.record_order_added();
        stats.record_order_added();
        stats.record_order_removed();
        stats.record_execution(50, 200, 1500);
        stats.record_execution(75, 180, 800);

        let serialized = serde_json::to_string(&stats).unwrap();
        let deserialized: PriceLevelStatistics = serde_json::from_str(&serialized).unwrap();

        assert_eq!(stats.orders_added(), deserialized.orders_added());
        assert_eq!(stats.orders_removed(), deserialized.orders_removed());
        assert_eq!(stats.orders_executed(), deserialized.orders_executed());
        assert_eq!(stats.quantity_executed(), deserialized.quantity_executed());
        assert_eq!(stats.value_executed(), deserialized.value_executed());
        assert_eq!(stats.last_execution_time, deserialized.last_execution_time);
        assert_eq!(stats.first_arrival_time, deserialized.first_arrival_time);
        assert_eq!(stats.sum_waiting_time, deserialized.sum_waiting_time);
    }

    #[test]
    fn test_statistics_visitor_missing_fields() {
        let incomplete_json = r#"{"orders_added": 5, "orders_removed": 2}"#;
        let result: Result<PriceLevelStatistics, _> = serde_json::from_str(incomplete_json);
        assert!(result.is_err());
    }
}
