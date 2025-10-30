use crate::errors::PriceLevelError;
use crate::order::Order;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

/// A snapshot of a price level in the order book. This struct provides a summary of the state of a specific price level
/// at a given point in time, including the price, visible and hidden quantities, order count, and a vector of the orders
/// at that level.
#[derive(Debug, Default, Clone)]
pub struct PriceLevelSnapshot {
    /// The price of this level.
    pub price: u64,
    /// Total display quantity at this level. This represents the sum of the display quantities of all orders at this price level.
    pub display_quantity: u64,
    /// Total reserve quantity at this level. This represents the sum of the reserve quantities of all orders at this price level.
    pub reserve_quantity: u64,
    /// Number of orders at this level.
    pub order_count: usize,
    /// Orders at this level.  This is a vector of `Arc<OrderType<()>>` representing each individual order at this price level.
    pub orders: Vec<Arc<Order<()>>>,
}

impl PriceLevelSnapshot {
    /// Create a new empty snapshot
    pub fn new(price: u64) -> Self {
        Self {
            price,
            display_quantity: 0,
            reserve_quantity: 0,
            order_count: 0,
            orders: Vec::new(),
        }
    }

    /// Get the total quantity (display + reserve) at this price level
    pub fn total_quantity(&self) -> u64 {
        self.display_quantity + self.reserve_quantity
    }

    /// Get an iterator over the orders in this snapshot
    pub fn iter_orders(&self) -> impl Iterator<Item = &Arc<Order<()>>> {
        self.orders.iter()
    }

    /// Recomputes aggregate fields (`display_quantity`, `reserve_quantity`, and `order_count`) based on current orders.
    pub fn refresh_aggregates(&mut self) {
        self.order_count = self.orders.len();

        let mut display_total: u64 = 0;
        let mut reserve_total: u64 = 0;

        for order in &self.orders {
            display_total = display_total.saturating_add(order.display_quantity());
            reserve_total = reserve_total.saturating_add(order.reserve_quantity());
        }

        self.display_quantity = display_total;
        self.reserve_quantity = reserve_total;
    }

    /// Get the visible quantity (deprecated: use display_quantity field instead)
    #[deprecated(since = "0.5.0", note = "Use display_quantity field instead")]
    pub fn visible_quantity(&self) -> u64 {
        self.display_quantity
    }

    /// Get the hidden quantity (deprecated: use reserve_quantity field instead)
    #[deprecated(since = "0.5.0", note = "Use reserve_quantity field instead")]
    pub fn hidden_quantity(&self) -> u64 {
        self.reserve_quantity
    }
}

/// Format version for checksum-enabled price level snapshots.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;

/// Serialized representation of a price level snapshot including checksum validation metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevelSnapshotPackage {
    /// Version of the serialized snapshot schema to support future migrations.
    pub version: u32,
    /// Captured snapshot data.
    pub snapshot: PriceLevelSnapshot,
    /// Hex-encoded checksum used to validate the snapshot integrity.
    pub checksum: String,
}

impl PriceLevelSnapshotPackage {
    /// Creates a new snapshot package computing the checksum for the provided snapshot.
    pub fn new(mut snapshot: PriceLevelSnapshot) -> Result<Self, PriceLevelError> {
        snapshot.refresh_aggregates();

        let checksum = Self::compute_checksum(&snapshot)?;

        Ok(Self {
            version: SNAPSHOT_FORMAT_VERSION,
            snapshot,
            checksum,
        })
    }

    /// Serializes the package to JSON.
    pub fn to_json(&self) -> Result<String, PriceLevelError> {
        serde_json::to_string(self).map_err(|error| PriceLevelError::SerializationError {
            message: error.to_string(),
        })
    }

    /// Deserializes a package from JSON.
    pub fn from_json(data: &str) -> Result<Self, PriceLevelError> {
        serde_json::from_str(data).map_err(|error| PriceLevelError::DeserializationError {
            message: error.to_string(),
        })
    }

    /// Validates the checksum contained in the package against the serialized snapshot data.
    pub fn validate(&self) -> Result<(), PriceLevelError> {
        if self.version != SNAPSHOT_FORMAT_VERSION {
            return Err(PriceLevelError::InvalidOperation {
                message: format!(
                    "Unsupported snapshot version: {} (expected {})",
                    self.version, SNAPSHOT_FORMAT_VERSION
                ),
            });
        }

        let computed = Self::compute_checksum(&self.snapshot)?;
        if computed != self.checksum {
            return Err(PriceLevelError::ChecksumMismatch {
                expected: self.checksum.clone(),
                actual: computed,
            });
        }

        Ok(())
    }

    /// Consumes the package after validating the checksum and returns the contained snapshot.
    pub fn into_snapshot(self) -> Result<PriceLevelSnapshot, PriceLevelError> {
        self.validate()?;
        Ok(self.snapshot)
    }

    fn compute_checksum(snapshot: &PriceLevelSnapshot) -> Result<String, PriceLevelError> {
        let payload =
            serde_json::to_vec(snapshot).map_err(|error| PriceLevelError::SerializationError {
                message: error.to_string(),
            })?;

        let mut hasher = Sha256::new();
        hasher.update(payload);

        let checksum_bytes = hasher.finalize();
        Ok(format!("{:x}", checksum_bytes))
    }
}

impl Serialize for PriceLevelSnapshot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("PriceLevelSnapshot", 5)?;

        state.serialize_field("price", &self.price)?;
        state.serialize_field("display_quantity", &self.display_quantity)?;
        state.serialize_field("reserve_quantity", &self.reserve_quantity)?;
        state.serialize_field("order_count", &self.order_count)?;

        let plain_orders: Vec<Order<()>> =
            self.orders.iter().map(|arc_order| **arc_order).collect();

        state.serialize_field("orders", &plain_orders)?;

        state.end()
    }
}

impl<'de> Deserialize<'de> for PriceLevelSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            Price,
            DisplayQuantity,
            ReserveQuantity,
            // Legacy fields for backward compatibility
            VisibleQuantity,
            HiddenQuantity,
            OrderCount,
            Orders,
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
                        formatter.write_str("`price`, `display_quantity`, `reserve_quantity`, `order_count`, or `orders`")
                    }

                    fn visit_str<E>(self, value: &str) -> Result<Field, E>
                    where
                        E: de::Error,
                    {
                        match value {
                            "price" => Ok(Field::Price),
                            "display_quantity" => Ok(Field::DisplayQuantity),
                            "reserve_quantity" => Ok(Field::ReserveQuantity),
                            // Legacy field names for backward compatibility
                            "visible_quantity" => Ok(Field::VisibleQuantity),
                            "hidden_quantity" => Ok(Field::HiddenQuantity),
                            "order_count" => Ok(Field::OrderCount),
                            "orders" => Ok(Field::Orders),
                            _ => Err(de::Error::unknown_field(
                                value,
                                &[
                                    "price",
                                    "display_quantity",
                                    "reserve_quantity",
                                    "visible_quantity",
                                    "hidden_quantity",
                                    "order_count",
                                    "orders",
                                ],
                            )),
                        }
                    }
                }

                deserializer.deserialize_identifier(FieldVisitor)
            }
        }

        struct PriceLevelSnapshotVisitor;

        impl<'de> Visitor<'de> for PriceLevelSnapshotVisitor {
            type Value = PriceLevelSnapshot;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct PriceLevelSnapshot")
            }

            fn visit_map<V>(self, mut map: V) -> Result<PriceLevelSnapshot, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut price = None;
                let mut display_quantity = None;
                let mut reserve_quantity = None;
                let mut order_count = None;
                let mut orders = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Price => {
                            if price.is_some() {
                                return Err(de::Error::duplicate_field("price"));
                            }
                            price = Some(map.next_value()?);
                        }
                        Field::DisplayQuantity => {
                            if display_quantity.is_some() {
                                return Err(de::Error::duplicate_field("display_quantity"));
                            }
                            display_quantity = Some(map.next_value()?);
                        }
                        Field::ReserveQuantity => {
                            if reserve_quantity.is_some() {
                                return Err(de::Error::duplicate_field("reserve_quantity"));
                            }
                            reserve_quantity = Some(map.next_value()?);
                        }
                        // Legacy field support for backward compatibility
                        Field::VisibleQuantity => {
                            if display_quantity.is_some() {
                                return Err(de::Error::duplicate_field("visible_quantity"));
                            }
                            display_quantity = Some(map.next_value()?);
                        }
                        Field::HiddenQuantity => {
                            if reserve_quantity.is_some() {
                                return Err(de::Error::duplicate_field("hidden_quantity"));
                            }
                            reserve_quantity = Some(map.next_value()?);
                        }
                        Field::OrderCount => {
                            if order_count.is_some() {
                                return Err(de::Error::duplicate_field("order_count"));
                            }
                            order_count = Some(map.next_value()?);
                        }
                        Field::Orders => {
                            if orders.is_some() {
                                return Err(de::Error::duplicate_field("orders"));
                            }
                            let plain_orders: Vec<Order<()>> = map.next_value()?;
                            orders = Some(plain_orders.into_iter().map(Arc::new).collect());
                        }
                    }
                }

                let price = price.ok_or_else(|| de::Error::missing_field("price"))?;
                let display_quantity =
                    display_quantity.ok_or_else(|| de::Error::missing_field("display_quantity"))?;
                let reserve_quantity =
                    reserve_quantity.ok_or_else(|| de::Error::missing_field("reserve_quantity"))?;
                let order_count =
                    order_count.ok_or_else(|| de::Error::missing_field("order_count"))?;
                let orders = orders.unwrap_or_default();

                Ok(PriceLevelSnapshot {
                    price,
                    display_quantity,
                    reserve_quantity,
                    order_count,
                    orders,
                })
            }
        }

        const FIELDS: &[&str] = &[
            "price",
            "display_quantity",
            "reserve_quantity",
            "order_count",
            "orders",
        ];
        deserializer.deserialize_struct("PriceLevelSnapshot", FIELDS, PriceLevelSnapshotVisitor)
    }
}

impl fmt::Display for PriceLevelSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PriceLevelSnapshot:price={};display_quantity={};reserve_quantity={};order_count={}",
            self.price, self.display_quantity, self.reserve_quantity, self.order_count
        )
    }
}

impl FromStr for PriceLevelSnapshot {
    type Err = PriceLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 || parts[0] != "PriceLevelSnapshot" {
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

        let parse_u64 = |field: &str, value: &str| -> Result<u64, PriceLevelError> {
            value
                .parse::<u64>()
                .map_err(|_| PriceLevelError::InvalidFieldValue {
                    field: field.to_string(),
                    value: value.to_string(),
                })
        };

        let parse_usize = |field: &str, value: &str| -> Result<usize, PriceLevelError> {
            value
                .parse::<usize>()
                .map_err(|_| PriceLevelError::InvalidFieldValue {
                    field: field.to_string(),
                    value: value.to_string(),
                })
        };

        // Parse fields
        let price_str = get_field("price")?;
        let price = parse_u64("price", price_str)?;

        let display_quantity_str = get_field("display_quantity")?;
        let display_quantity = parse_u64("display_quantity", display_quantity_str)?;

        let reserve_quantity_str = get_field("reserve_quantity")?;
        let reserve_quantity = parse_u64("reserve_quantity", reserve_quantity_str)?;

        let order_count_str = get_field("order_count")?;
        let order_count = parse_usize("order_count", order_count_str)?;

        // Create a new snapshot - note that orders cannot be serialized/deserialized in this simple format
        Ok(PriceLevelSnapshot {
            price,
            display_quantity,
            reserve_quantity,
            order_count,
            orders: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::errors::PriceLevelError;
    use crate::order::{OrderCommon, OrderId, Order, Side, TimeInForce};
    use crate::price_level::snapshot::SNAPSHOT_FORMAT_VERSION;
    use crate::price_level::{PriceLevelSnapshot, PriceLevelSnapshotPackage};
    use serde_json::Value;
    use std::str::FromStr;
    use std::sync::Arc;

    fn create_sample_orders() -> Vec<Arc<Order<()>>> {
        vec![
            Arc::new(Order::Standard {
                common: OrderCommon {
                    id: OrderId::from_u64(1),
                    price: 1000,
                    display_quantity: 10,
                    side: Side::Buy,
                    timestamp: 1616823000000,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
            }),
            Arc::new(Order::IcebergOrder {
                common: OrderCommon {
                    id: OrderId::from_u64(2),
                    price: 1000,
                    display_quantity: 5,
                    side: Side::Buy,
                    timestamp: 1616823000001,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
                reserve_quantity: 15,
            }),
        ]
    }

    #[test]
    fn test_snapshot_package_roundtrip() {
        let mut snapshot = PriceLevelSnapshot::new(42);
        snapshot.orders = create_sample_orders();
        snapshot.refresh_aggregates();

        let package =
            PriceLevelSnapshotPackage::new(snapshot.clone()).expect("Failed to create package");

        assert_eq!(package.version, SNAPSHOT_FORMAT_VERSION);
        package.validate().expect("Package validation failed");

        let json = package.to_json().expect("Failed to serialize package");
        let restored_package =
            PriceLevelSnapshotPackage::from_json(&json).expect("Failed to deserialize package");

        restored_package
            .validate()
            .expect("Checksum validation should succeed");

        let restored_snapshot = restored_package
            .into_snapshot()
            .expect("Snapshot extraction failed");

        assert_eq!(restored_snapshot.price, snapshot.price);
        assert_eq!(restored_snapshot.order_count, snapshot.order_count);
        assert_eq!(
            restored_snapshot.display_quantity,
            snapshot.display_quantity
        );
        assert_eq!(
            restored_snapshot.reserve_quantity,
            snapshot.reserve_quantity
        );
        assert_eq!(restored_snapshot.orders.len(), snapshot.orders.len());
    }

    #[test]
    fn test_snapshot_package_checksum_mismatch() {
        let mut snapshot = PriceLevelSnapshot::new(99);
        snapshot.orders = create_sample_orders();
        snapshot.refresh_aggregates();

        let package = PriceLevelSnapshotPackage::new(snapshot).expect("Failed to create package");
        let json = package.to_json().expect("Failed to serialize package");

        let mut value: Value = serde_json::from_str(&json).expect("JSON parsing failed");
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "checksum".to_string(),
                Value::String("deadbeef".to_string()),
            );
        }

        let tampered_json = serde_json::to_string(&value).expect("JSON serialization failed");

        let tampered_package = PriceLevelSnapshotPackage::from_json(&tampered_json)
            .expect("Deserialization should still succeed");

        let err = tampered_package
            .validate()
            .expect_err("Checksum mismatch expected");
        assert!(matches!(err, PriceLevelError::ChecksumMismatch { .. }));
    }

    #[test]
    fn test_new() {
        let snapshot = PriceLevelSnapshot::new(1000);
        assert_eq!(snapshot.price, 1000);
        assert_eq!(snapshot.display_quantity, 0);
        assert_eq!(snapshot.reserve_quantity, 0);
        assert_eq!(snapshot.order_count, 0);
        assert!(snapshot.orders.is_empty());
    }

    #[test]
    fn test_default() {
        let snapshot = PriceLevelSnapshot::default();
        assert_eq!(snapshot.price, 0);
        assert_eq!(snapshot.display_quantity, 0);
        assert_eq!(snapshot.reserve_quantity, 0);
        assert_eq!(snapshot.order_count, 0);
        assert!(snapshot.orders.is_empty());
    }

    #[test]
    fn test_total_quantity() {
        let mut snapshot = PriceLevelSnapshot::new(1000);
        snapshot.display_quantity = 50;
        snapshot.reserve_quantity = 150;
        assert_eq!(snapshot.total_quantity(), 200);
    }

    #[test]
    fn test_iter_orders() {
        let mut snapshot = PriceLevelSnapshot::new(1000);
        let orders = create_sample_orders();
        snapshot.orders = orders.clone();
        snapshot.order_count = orders.len();

        let collected: Vec<_> = snapshot.iter_orders().collect();
        assert_eq!(collected.len(), 2);

        // Verify first order
        if let Order::Standard {
            common: OrderCommon { id, .. },
        } = **collected[0]
        {
            assert_eq!(id, OrderId::from_u64(1));
        } else {
            panic!("Expected StandardOrder");
        }

        // Verify second order
        if let Order::IcebergOrder {
            common: OrderCommon { id, .. },
            ..
        } = **collected[1]
        {
            assert_eq!(id, OrderId::from_u64(2));
        } else {
            panic!("Expected IcebergOrder");
        }
    }

    #[test]
    fn test_clone() {
        let mut original = PriceLevelSnapshot::new(1000);
        original.display_quantity = 50;
        original.reserve_quantity = 150;
        original.order_count = 2;
        original.orders = create_sample_orders();

        let cloned = original.clone();
        assert_eq!(cloned.price, 1000);
        assert_eq!(cloned.display_quantity, 50);
        assert_eq!(cloned.reserve_quantity, 150);
        assert_eq!(cloned.order_count, 2);
        assert_eq!(cloned.orders.len(), 2);
    }

    #[test]
    fn test_display() {
        let mut snapshot = PriceLevelSnapshot::new(1000);
        snapshot.display_quantity = 50;
        snapshot.reserve_quantity = 150;
        snapshot.order_count = 2;

        let display_str = snapshot.to_string();
        assert!(display_str.contains("price=1000"));
        assert!(display_str.contains("display_quantity=50"));
        assert!(display_str.contains("reserve_quantity=150"));
        assert!(display_str.contains("order_count=2"));
    }

    #[test]
    fn test_from_str() {
        let input =
            "PriceLevelSnapshot:price=1000;display_quantity=50;reserve_quantity=150;order_count=2";
        let snapshot = PriceLevelSnapshot::from_str(input).unwrap();

        assert_eq!(snapshot.price, 1000);
        assert_eq!(snapshot.display_quantity, 50);
        assert_eq!(snapshot.reserve_quantity, 150);
        assert_eq!(snapshot.order_count, 2);
        assert!(snapshot.orders.is_empty()); // Orders can't be parsed from string representation
    }

    #[test]
    fn test_from_str_invalid_format() {
        let input = "InvalidFormat";
        let result = PriceLevelSnapshot::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_str_missing_field() {
        let input = "PriceLevelSnapshot:price=1000;display_quantity=50;reserve_quantity=150";
        let result = PriceLevelSnapshot::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_str_invalid_field_value() {
        let input = "PriceLevelSnapshot:price=invalid;display_quantity=50;reserve_quantity=150;order_count=2";
        let result = PriceLevelSnapshot::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip_display_fromstr() {
        let mut original = PriceLevelSnapshot::new(1000);
        original.display_quantity = 50;
        original.reserve_quantity = 150;
        original.order_count = 2;

        let string_representation = original.to_string();
        let parsed = PriceLevelSnapshot::from_str(&string_representation).unwrap();

        assert_eq!(parsed.price, original.price);
        assert_eq!(parsed.display_quantity, original.display_quantity);
        assert_eq!(parsed.reserve_quantity, original.reserve_quantity);
        assert_eq!(parsed.order_count, original.order_count);
    }

    // In price_level/snapshot.rs test module or in a separate test file

    #[test]
    fn test_snapshot_serialization_fields() {
        // Create a snapshot with specific field values
        let mut snapshot = PriceLevelSnapshot::new(10000);
        snapshot.display_quantity = 200;
        snapshot.reserve_quantity = 300;
        snapshot.order_count = 5;

        // Add some orders (empty for now, we'll test orders separately)

        // Serialize to JSON
        let serialized = serde_json::to_string(&snapshot).unwrap();

        // Check the serialized fields
        assert!(serialized.contains("\"price\":10000"));
        assert!(serialized.contains("\"display_quantity\":200"));
        assert!(serialized.contains("\"reserve_quantity\":300"));
        assert!(serialized.contains("\"order_count\":5"));
        assert!(serialized.contains("\"orders\":[]"));
    }

    #[test]
    fn test_snapshot_deserializer_duplicate_fields() {
        // Test with duplicate field
        let json = r#"{
        "price": 10000,
        "display_quantity": 200,
        "reserve_quantity": 300,
        "order_count": 5,
        "price": 20000,
        "orders": []
    }"#;

        // Should fail due to duplicate field
        let result = serde_json::from_str::<PriceLevelSnapshot>(json);
        assert!(result.is_err());

        // Error should mention duplicate field
        let err = result.unwrap_err().to_string();
        assert!(err.contains("duplicate field"));
    }

    #[test]
    fn test_snapshot_visitor_implementation() {
        // Testing the visitor by providing various field values
        let json = r#"{
        "price": 10000,
        "display_quantity": 200,
        "reserve_quantity": 300,
        "order_count": 5,
        "orders": []
    }"#;

        let snapshot: PriceLevelSnapshot = serde_json::from_str(json).unwrap();

        assert_eq!(snapshot.price, 10000);
        assert_eq!(snapshot.display_quantity, 200);
        assert_eq!(snapshot.reserve_quantity, 300);
        assert_eq!(snapshot.order_count, 5);
        assert!(snapshot.orders.is_empty());
    }

    #[test]
    fn test_snapshot_with_actual_orders() {
        fn create_standard_order(id: u64, price: u64, quantity: u64) -> Order<()> {
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

        fn create_iceberg_order(
            id: u64,
            price: u64,
            visible_quantity: u64,
            hidden_quantity: u64,
        ) -> Order<()> {
            Order::<()>::IcebergOrder {
                common: OrderCommon {
                    id: OrderId::from_u64(id),
                    price,
                    display_quantity: visible_quantity,
                    side: Side::Buy,
                    timestamp: 1616823000000,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
                reserve_quantity: hidden_quantity,
            }
        }
        // Create a snapshot with orders
        let mut snapshot = PriceLevelSnapshot::new(10000);
        snapshot.display_quantity = 150;
        snapshot.reserve_quantity = 250;
        snapshot.order_count = 2;

        let orders = vec![
            Arc::new(create_standard_order(1, 10000, 100)),
            Arc::new(create_iceberg_order(2, 10000, 50, 250)),
        ];

        snapshot.orders = orders;

        // Serialize to JSON
        let serialized = serde_json::to_string(&snapshot).unwrap();

        // Check serialized fields and orders
        assert!(serialized.contains("\"price\":10000"));
        assert!(serialized.contains("\"visible_quantity\":150"));
        assert!(serialized.contains("\"hidden_quantity\":250"));
        assert!(serialized.contains("\"order_count\":2"));
        assert!(serialized.contains("\"orders\":["));
        assert!(serialized.contains("\"Standard\":{"));
        assert!(serialized.contains("\"IcebergOrder\":{"));

        // Deserialize back
        let deserialized: PriceLevelSnapshot = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.price, 10000);
        assert_eq!(deserialized.display_quantity, 150);
        assert_eq!(deserialized.reserve_quantity, 250);
        assert_eq!(deserialized.order_count, 2);
        assert_eq!(deserialized.orders.len(), 2);

        // Verify order types
        if let Order::Standard {
            common:
                OrderCommon {
                    id,
                    display_quantity: quantity,
                    ..
                },
        } = &*deserialized.orders[0]
        {
            assert_eq!(*id, OrderId::from_u64(1));
            assert_eq!(*quantity, 100);
        } else {
            panic!("Expected Standard order");
        }

        if let Order::IcebergOrder {
            common:
                OrderCommon {
                    id,
                    display_quantity: visible_quantity,
                    ..
                },
            reserve_quantity: hidden_quantity,
            ..
        } = &*deserialized.orders[1]
        {
            assert_eq!(*id, OrderId::from_u64(2));
            assert_eq!(*visible_quantity, 50);
            assert_eq!(*hidden_quantity, 250);
        } else {
            panic!("Expected IcebergOrder");
        }
    }
}

#[cfg(test)]
mod pricelevel_snapshot_serialization_tests {
    use crate::order::{OrderCommon, OrderId, Order, Side, TimeInForce};
    use crate::price_level::PriceLevelSnapshot;

    use std::str::FromStr;
    use std::sync::Arc;

    // Helper function to create sample orders for testing
    fn create_sample_orders() -> Vec<Arc<Order<()>>> {
        vec![
            Arc::new(Order::Standard {
                common: OrderCommon {
                    id: OrderId::from_u64(1),
                    price: 1000,
                    display_quantity: 10,
                    side: Side::Buy,
                    timestamp: 1616823000000,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
            }),
            Arc::new(Order::IcebergOrder {
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
            }),
            Arc::new(Order::PostOnly {
                common: OrderCommon {
                    id: OrderId::from_u64(3),
                    price: 1000,
                    display_quantity: 8,
                    side: Side::Buy,
                    timestamp: 1616823000002,
                    time_in_force: TimeInForce::Ioc,
                    extra_fields: (),
                },
            }),
        ]
    }

    // Helper function to create a sample snapshot for testing
    fn create_sample_snapshot() -> PriceLevelSnapshot {
        let mut snapshot = PriceLevelSnapshot::new(1000);
        snapshot.display_quantity = 15; // 10 + 5 (first two orders)
        snapshot.reserve_quantity = 15; // hidden quantity from iceberg order
        snapshot.order_count = 3;
        snapshot.orders = create_sample_orders();
        snapshot
    }

    #[test]
    fn test_snapshot_json_serialization() {
        let snapshot = create_sample_snapshot();

        // Serialize to JSON
        let json = serde_json::to_string(&snapshot)
            .expect("Failed to serialize PriceLevelSnapshot to JSON");

        // Verify basic JSON properties
        assert!(json.contains("\"price\":1000"));
        assert!(json.contains("\"visible_quantity\":15"));
        assert!(json.contains("\"hidden_quantity\":15"));
        assert!(json.contains("\"order_count\":3"));

        // Verify orders array
        assert!(json.contains("\"orders\":["));

        // Check for order details
        assert!(json.contains("\"Standard\":{"));
        assert!(json.contains("\"id\":\"00000000-0000-0001-0000-000000000000\""));
        assert!(json.contains("\"IcebergOrder\":{"));
        assert!(json.contains("\"display_quantity\":5"));
        assert!(json.contains("\"reserve_quantity\":15"));
        assert!(json.contains("\"PostOnly\":{"));
    }

    #[test]
    fn test_snapshot_json_deserialization() {
        let snapshot = create_sample_snapshot();

        // Serialize to JSON
        let json =
            serde_json::to_string(&snapshot).expect("Failed to serialize PriceLevelSnapshot");

        // Deserialize back to PriceLevelSnapshot
        let deserialized: PriceLevelSnapshot = serde_json::from_str(&json)
            .expect("Failed to deserialize PriceLevelSnapshot from JSON");

        // Verify basic fields
        assert_eq!(deserialized.price, 1000);
        assert_eq!(deserialized.display_quantity, 15);
        assert_eq!(deserialized.reserve_quantity, 15);
        assert_eq!(deserialized.order_count, 3);

        // Verify orders array length
        assert_eq!(deserialized.orders.len(), 3);

        // Check specific order details
        let standard_order = &deserialized.orders[0];
        match **standard_order {
            Order::Standard {
                common:
                    OrderCommon {
                        id,
                        price,
                        display_quantity: quantity,
                        side,
                        ..
                    },
            } => {
                assert_eq!(id, OrderId::from_u64(1));
                assert_eq!(price, 1000);
                assert_eq!(quantity, 10);
                assert_eq!(side, Side::Buy);
            }
            _ => panic!("Expected Standard order"),
        }

        let iceberg_order = &deserialized.orders[1];
        match **iceberg_order {
            Order::IcebergOrder {
                common:
                    OrderCommon {
                        id,
                        display_quantity,
                        side,
                        ..
                    },
                reserve_quantity,
                ..
            } => {
                assert_eq!(id, OrderId::from_u64(2));
                assert_eq!(display_quantity, 5);
                assert_eq!(reserve_quantity, 15);
                assert_eq!(side, Side::Sell);
            }
            _ => panic!("Expected IcebergOrder"),
        }

        let post_only_order = &deserialized.orders[2];
        match **post_only_order {
            Order::<()>::PostOnly {
                common:
                    OrderCommon {
                        id,
                        display_quantity: quantity,
                        side,
                        ..
                    },
            } => {
                assert_eq!(id, OrderId::from_u64(3));
                assert_eq!(quantity, 8);
                assert_eq!(side, Side::Buy);
            }
            _ => panic!("Expected PostOnly order"),
        }
    }

    #[test]
    fn test_snapshot_string_format_serialization() {
        let snapshot = create_sample_snapshot();

        // Convert to string representation
        let display_str = snapshot.to_string();

        // Verify string format
        assert!(display_str.starts_with("PriceLevelSnapshot:"));
        assert!(display_str.contains("price=1000"));
        assert!(display_str.contains("display_quantity=15"));
        assert!(display_str.contains("reserve_quantity=15"));
        assert!(display_str.contains("order_count=3"));

        // Note: The string format doesn't include orders as shown in the FromStr implementation
    }

    #[test]
    fn test_snapshot_string_format_deserialization() {
        // Create string representation
        let input =
            "PriceLevelSnapshot:price=1000;display_quantity=15;reserve_quantity=15;order_count=3";

        // Parse from string
        let snapshot =
            PriceLevelSnapshot::from_str(input).expect("Failed to parse PriceLevelSnapshot");

        // Verify basic fields
        assert_eq!(snapshot.price, 1000);
        assert_eq!(snapshot.display_quantity, 15);
        assert_eq!(snapshot.reserve_quantity, 15);
        assert_eq!(snapshot.order_count, 3);

        // Orders array should be empty when deserialized from string format (per FromStr implementation)
        assert!(snapshot.orders.is_empty());
    }

    #[test]
    fn test_snapshot_string_format_invalid_inputs() {
        // Test missing price field
        let input = "PriceLevelSnapshot:display_quantity=15;reserve_quantity=15;order_count=3";
        let result = PriceLevelSnapshot::from_str(input);
        assert!(result.is_err());

        // Test invalid prefix
        let input =
            "InvalidPrefix:price=1000;display_quantity=15;reserve_quantity=15;order_count=3";
        let result = PriceLevelSnapshot::from_str(input);
        assert!(result.is_err());

        // Test invalid field value
        let input = "PriceLevelSnapshot:price=invalid;display_quantity=15;reserve_quantity=15;order_count=3";
        let result = PriceLevelSnapshot::from_str(input);
        assert!(result.is_err());

        // Test missing field separator
        let input =
            "PriceLevelSnapshot:price=1000display_quantity=15;reserve_quantity=15;order_count=3";
        let result = PriceLevelSnapshot::from_str(input);
        assert!(result.is_err());

        // Test with unknown field
        let input = "PriceLevelSnapshot:price=1000;display_quantity=15;reserve_quantity=15;order_count=3;unknown_field=value";
        let result = PriceLevelSnapshot::from_str(input);
        // This should still succeed as FromStr implementation doesn't validate for unknown fields
        assert!(result.is_ok());
    }

    #[test]
    fn test_snapshot_string_format_roundtrip() {
        // Create a snapshot with only basic fields (no orders)
        let mut original = PriceLevelSnapshot::new(1000);
        original.display_quantity = 15;
        original.reserve_quantity = 15;
        original.order_count = 3;

        // Convert to string
        let string_representation = original.to_string();

        // Parse back to snapshot
        let parsed = PriceLevelSnapshot::from_str(&string_representation)
            .expect("Failed to parse PriceLevelSnapshot");

        // Verify all fields match
        assert_eq!(parsed.price, original.price);
        assert_eq!(parsed.display_quantity, original.display_quantity);
        assert_eq!(parsed.reserve_quantity, original.reserve_quantity);
        assert_eq!(parsed.order_count, original.order_count);
    }

    #[test]
    fn test_snapshot_edge_cases() {
        // Test with zero values
        let mut snapshot = PriceLevelSnapshot::new(0);
        snapshot.display_quantity = 0;
        snapshot.reserve_quantity = 0;
        snapshot.order_count = 0;

        let json = serde_json::to_string(&snapshot).expect("Failed to serialize");
        let deserialized: PriceLevelSnapshot =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.price, 0);
        assert_eq!(deserialized.display_quantity, 0);
        assert_eq!(deserialized.reserve_quantity, 0);
        assert_eq!(deserialized.order_count, 0);

        // Test with maximum values
        let mut snapshot = PriceLevelSnapshot::new(u64::MAX);
        snapshot.display_quantity = u64::MAX;
        snapshot.reserve_quantity = u64::MAX;
        snapshot.order_count = usize::MAX;

        let json = serde_json::to_string(&snapshot).expect("Failed to serialize max values");
        let deserialized: PriceLevelSnapshot =
            serde_json::from_str(&json).expect("Failed to deserialize max values");

        assert_eq!(deserialized.price, u64::MAX);
        assert_eq!(deserialized.display_quantity, u64::MAX);
        assert_eq!(deserialized.reserve_quantity, u64::MAX);
        assert_eq!(deserialized.order_count, usize::MAX);
    }

    #[test]
    fn test_snapshot_deserialization_unknown_field() {
        // Create JSON with an unknown field "unknown_field"
        let json = r#"{
            "price": 1000,
            "display_quantity": 15,
            "reserve_quantity": 15,
            "order_count": 3,
            "orders": [],
            "unknown_field": "some value"
        }"#;

        // Attempt to deserialize - this should fail because of the unknown field
        let result = serde_json::from_str::<PriceLevelSnapshot>(json);

        // Verify that the error is of the expected type
        assert!(result.is_err());
        let err = result.unwrap_err();
        let err_string = err.to_string();

        // Verify the error message mentions the unknown field
        assert!(err_string.contains("unknown field"));
        assert!(err_string.contains("unknown_field"));

        // Verify the error message mentions the expected fields
        assert!(err_string.contains("price"));
        assert!(err_string.contains("display_quantity"));
        assert!(err_string.contains("reserve_quantity"));
        assert!(err_string.contains("order_count"));
        assert!(err_string.contains("orders"));
    }

    #[test]
    fn test_snapshot_empty_orders() {
        // Test with an empty orders array
        let mut snapshot = PriceLevelSnapshot::new(1000);
        snapshot.display_quantity = 15;
        snapshot.reserve_quantity = 15;
        snapshot.order_count = 0;
        snapshot.orders = Vec::new();

        let json = serde_json::to_string(&snapshot).expect("Failed to serialize");
        let deserialized: PriceLevelSnapshot =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.price, 1000);
        assert_eq!(deserialized.orders.len(), 0);
    }

    #[test]
    fn test_snapshot_with_many_order_types() {
        // Create a snapshot with all supported order types
        let mut snapshot = PriceLevelSnapshot::new(1000);

        // Add sample orders of different types
        snapshot.orders = vec![
            // Standard order
            Arc::new(Order::Standard {
                common: OrderCommon {
                    id: OrderId::from_u64(1),
                    price: 1000,
                    display_quantity: 10,
                    side: Side::Buy,
                    timestamp: 1616823000000,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
            }),
            // Iceberg order
            Arc::new(Order::IcebergOrder {
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
            }),
            // Post-only order
            Arc::new(Order::PostOnly {
                common: OrderCommon {
                    id: OrderId::from_u64(3),
                    price: 1000,
                    display_quantity: 8,
                    side: Side::Buy,
                    timestamp: 1616823000002,
                    time_in_force: TimeInForce::Ioc,
                    extra_fields: (),
                },
            }),
            // Fill-or-kill order (as Standard with FOK time-in-force)
            Arc::new(Order::Standard {
                common: OrderCommon {
                    id: OrderId::from_u64(4),
                    price: 1000,
                    display_quantity: 12,
                    side: Side::Buy,
                    timestamp: 1616823000003,
                    time_in_force: TimeInForce::Fok,
                    extra_fields: (),
                },
            }),
            // Good-till-date order (as Standard with GTD time-in-force)
            Arc::new(Order::Standard {
                common: OrderCommon {
                    id: OrderId::from_u64(5),
                    price: 1000,
                    display_quantity: 7,
                    side: Side::Sell,
                    timestamp: 1616823000004,
                    time_in_force: TimeInForce::Gtd(1617000000000),
                    extra_fields: (),
                },
            }),
            // Reserve order
            Arc::new(Order::ReserveOrder {
                common: OrderCommon {
                    id: OrderId::from_u64(6),
                    price: 1000,
                    display_quantity: 3,
                    side: Side::Buy,
                    timestamp: 1616823000005,
                    time_in_force: TimeInForce::Gtc,
                    extra_fields: (),
                },
                reserve_quantity: 12,
                replenish_threshold: 1,
                replenish_amount: None,
                auto_replenish: true,
            }),
        ];

        snapshot.order_count = snapshot.orders.len();
        snapshot.display_quantity = 45; // Sum of all display quantities
        snapshot.reserve_quantity = 27; // Sum of all reserve quantities

        // Serialize to JSON
        let json = serde_json::to_string(&snapshot).expect("Failed to serialize complex snapshot");

        // Deserialize back
        let deserialized: PriceLevelSnapshot =
            serde_json::from_str(&json).expect("Failed to deserialize complex snapshot");

        // Verify basic fields
        assert_eq!(deserialized.price, 1000);
        assert_eq!(deserialized.display_quantity, 45);
        assert_eq!(deserialized.reserve_quantity, 27);
        assert_eq!(deserialized.order_count, 6);
        assert_eq!(deserialized.orders.len(), 6);

        // Verify specific order types were preserved
        let order_types = deserialized
            .orders
            .iter()
            .map(|order| match **order {
                Order::Standard { .. } => "Standard",
                Order::IcebergOrder { .. } => "IcebergOrder",
                Order::PostOnly { .. } => "PostOnly",
                Order::ReserveOrder { .. } => "ReserveOrder",
                _ => "Other",
            })
            .collect::<Vec<_>>();

        // Count the occurrences of each order type
        let standard_count = order_types.iter().filter(|&&t| t == "Standard").count();
        let iceberg_count = order_types.iter().filter(|&&t| t == "IcebergOrder").count();
        let post_only_count = order_types.iter().filter(|&&t| t == "PostOnly").count();
        let reserve_count = order_types.iter().filter(|&&t| t == "ReserveOrder").count();

        // Verify we have the expected number of each order type
        assert_eq!(standard_count, 3); // 1 standard + 1 FOK + 1 GTD
        assert_eq!(iceberg_count, 1);
        assert_eq!(post_only_count, 1);
        assert_eq!(reserve_count, 1);

        // Check a few specific properties to ensure proper deserialization
        let reserve_order = deserialized
            .orders
            .iter()
            .find(|order| matches!(***order, Order::ReserveOrder { .. }))
            .expect("Reserve order not found");

        if let Order::ReserveOrder {
            replenish_threshold,
            auto_replenish,
            ..
        } = **reserve_order
        {
            assert_eq!(replenish_threshold, 1);
            assert!(auto_replenish);
        }

        let gtd_order = deserialized
            .orders
            .iter()
            .find(|order| {
                matches!(
                    ***order,
                    Order::Standard {
                        common: OrderCommon {
                            time_in_force: TimeInForce::Gtd(_),
                            ..
                        },
                    }
                )
            })
            .expect("GTD order not found");

        if let Order::Standard {
            common:
                OrderCommon {
                    time_in_force: TimeInForce::Gtd(expiry),
                    ..
                },
        } = **gtd_order
        {
            assert_eq!(expiry, 1617000000000);
        }
    }
}
