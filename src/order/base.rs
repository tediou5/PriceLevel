//! Base order definitions

use crate::errors::PriceLevelError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use ulid::Ulid;
use uuid::Uuid;

/// Represents the side of an order
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    /// Buy side (bids)
    #[serde(rename(serialize = "BUY"))]
    #[serde(alias = "buy", alias = "Buy", alias = "BUY")]
    Buy,
    /// Sell side (asks)
    #[serde(rename(serialize = "SELL"))]
    #[serde(alias = "sell", alias = "Sell", alias = "SELL")]
    Sell,
}

impl Side {
    /// Returns the opposite side of the order.
    ///
    /// # Examples
    ///
    /// ```
    /// use pricelevel::Side;
    /// let buy_side = Side::Buy;
    /// let sell_side = buy_side.opposite();
    /// assert_eq!(sell_side, Side::Sell);
    ///
    /// let sell_side = Side::Sell;
    /// let buy_side = sell_side.opposite();
    /// assert_eq!(buy_side, Side::Buy);
    /// ```
    pub fn opposite(&self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

impl FromStr for Side {
    type Err = PriceLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "BUY" => Ok(Side::Buy),
            "SELL" => Ok(Side::Sell),
            _ => Err(PriceLevelError::ParseError {
                message: "Failed to parse Side".to_string(),
            }),
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Side::Buy => write!(f, "BUY"),
            Side::Sell => write!(f, "SELL"),
        }
    }
}

/// Represents a unique identifier for an order in the trading system.
///
/// This enum supports two different ID formats to provide flexibility
/// in order identification and tracking across different systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderId {
    /// UUID (Universally Unique Identifier) format
    /// A 128-bit identifier that is globally unique across space and time
    Uuid(Uuid),

    /// ULID (Universally Unique Lexicographically Sortable Identifier) format
    /// A 128-bit identifier that is lexicographically sortable and globally unique
    Ulid(Ulid),
}

impl FromStr for OrderId {
    type Err = PriceLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Try UUID first (has hyphens), then ULID
        if let Ok(uuid) = Uuid::from_str(s) {
            Ok(OrderId::Uuid(uuid))
        } else if let Ok(ulid) = Ulid::from_string(s) {
            Ok(OrderId::Ulid(ulid))
        } else {
            Err(PriceLevelError::ParseError {
                message: format!("Failed to parse OrderId as UUID or ULID: {s}"),
            })
        }
    }
}

impl fmt::Display for OrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrderId::Uuid(uuid) => write!(f, "{}", uuid),
            OrderId::Ulid(ulid) => write!(f, "{}", ulid),
        }
    }
}

// Custom serialization to maintain backward compatibility
impl Serialize for OrderId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// Custom deserialization to maintain backward compatibility
impl<'de> Deserialize<'de> for OrderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        OrderId::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl Default for OrderId {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderId {
    /// Create a new random OrderId (defaults to ULID for better sortability)
    pub fn new() -> Self {
        OrderId::Ulid(Ulid::new())
    }

    /// Create a new UUID-based OrderId
    pub fn new_uuid() -> Self {
        OrderId::Uuid(Uuid::new_v4())
    }

    /// Create a new ULID-based OrderId
    pub fn new_ulid() -> Self {
        OrderId::Ulid(Ulid::new())
    }

    /// Create a nil OrderId (UUID format)
    pub fn nil() -> Self {
        OrderId::Uuid(Uuid::nil())
    }

    /// Create from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        OrderId::Uuid(uuid)
    }

    /// Create from an existing ULID
    pub fn from_ulid(ulid: Ulid) -> Self {
        OrderId::Ulid(ulid)
    }

    /// Get as bytes (both UUID and ULID are 16 bytes)
    pub fn as_bytes(&self) -> [u8; 16] {
        match self {
            OrderId::Uuid(uuid) => *uuid.as_bytes(),
            OrderId::Ulid(ulid) => ulid.to_bytes(),
        }
    }

    /// For backward compatibility with code still using u64 IDs
    pub fn from_u64(id: u64) -> Self {
        let bytes = [
            ((id >> 56) & 0xFF) as u8,
            ((id >> 48) & 0xFF) as u8,
            ((id >> 40) & 0xFF) as u8,
            ((id >> 32) & 0xFF) as u8,
            ((id >> 24) & 0xFF) as u8,
            ((id >> 16) & 0xFF) as u8,
            ((id >> 8) & 0xFF) as u8,
            (id & 0xFF) as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        OrderId::Uuid(Uuid::from_bytes(bytes))
    }
}

#[cfg(test)]
mod tests_side {
    use crate::order::Side;

    #[test]
    fn test_side_equality() {
        assert_eq!(Side::Buy, Side::Buy);
        assert_eq!(Side::Sell, Side::Sell);
        assert_ne!(Side::Buy, Side::Sell);
    }

    #[test]
    fn test_side_clone() {
        let buy = Side::Buy;
        let cloned_buy = buy;
        assert_eq!(buy, cloned_buy);

        let sell = Side::Sell;
        let cloned_sell = sell;
        assert_eq!(sell, cloned_sell);
    }

    #[test]
    fn test_serialize_to_uppercase() {
        assert_eq!(serde_json::to_string(&Side::Buy).unwrap(), "\"BUY\"");
        assert_eq!(serde_json::to_string(&Side::Sell).unwrap(), "\"SELL\"");
    }

    #[test]
    fn test_deserialize_uppercase() {
        assert_eq!(serde_json::from_str::<Side>("\"BUY\"").unwrap(), Side::Buy);
        assert_eq!(
            serde_json::from_str::<Side>("\"SELL\"").unwrap(),
            Side::Sell
        );
    }

    #[test]
    fn test_deserialize_lowercase() {
        assert_eq!(serde_json::from_str::<Side>("\"buy\"").unwrap(), Side::Buy);
        assert_eq!(
            serde_json::from_str::<Side>("\"sell\"").unwrap(),
            Side::Sell
        );
    }

    #[test]
    fn test_deserialize_capitalized() {
        assert_eq!(serde_json::from_str::<Side>("\"Buy\"").unwrap(), Side::Buy);
        assert_eq!(
            serde_json::from_str::<Side>("\"Sell\"").unwrap(),
            Side::Sell
        );
    }

    #[test]
    fn test_round_trip_serialization() {
        let sides = vec![Side::Buy, Side::Sell];

        for side in sides {
            let serialized = serde_json::to_string(&side).unwrap();
            let deserialized: Side = serde_json::from_str(&serialized).unwrap();
            assert_eq!(side, deserialized);
        }
    }

    #[test]
    fn test_invalid_deserialization() {
        assert!(serde_json::from_str::<Side>("\"INVALID\"").is_err());
        assert!(serde_json::from_str::<Side>("\"BUYING\"").is_err());
        assert!(serde_json::from_str::<Side>("\"SELLING\"").is_err());
        assert!(serde_json::from_str::<Side>("123").is_err());
        assert!(serde_json::from_str::<Side>("null").is_err());
    }

    #[test]
    fn test_from_string() {
        assert_eq!("BUY".parse::<Side>().unwrap(), Side::Buy);
        assert_eq!("SELL".parse::<Side>().unwrap(), Side::Sell);
        assert_eq!("buy".parse::<Side>().unwrap(), Side::Buy);
        assert_eq!("sell".parse::<Side>().unwrap(), Side::Sell);
    }

    #[test]
    fn test_serialized_size() {
        assert_eq!(serde_json::to_string(&Side::Buy).unwrap().len(), 5); // "BUY"
        assert_eq!(serde_json::to_string(&Side::Sell).unwrap().len(), 6); // "SELL"
    }
}

#[cfg(test)]
mod tests_orderid {
    use crate::Side;
    use crate::order::OrderId;
    use std::str::FromStr;
    use uuid::Uuid;

    #[test]
    fn test_order_id_creation() {
        // Create using from_u64 for backward compatibility
        let id = OrderId::from_u64(12345);
        // Test that it's a valid OrderId (can't access internal structure directly)
        assert_eq!(id, OrderId::from_u64(12345));

        // Create random UUIDs
        let id1 = OrderId::new();
        let id2 = OrderId::new();
        assert_ne!(id1, id2); // Random UUIDs should be different

        // Create from existing UUID
        let uuid = Uuid::new_v4();
        let id = OrderId::from_uuid(uuid);
        assert_eq!(id, OrderId::Uuid(uuid));

        // Create nil UUID
        let nil_id = OrderId::nil();
        assert_eq!(nil_id, OrderId::Uuid(Uuid::nil()));
    }

    #[test]
    fn test_order_id_equality() {
        let id1 = OrderId::from_u64(12345);
        let id2 = OrderId::from_u64(12345);
        let id3 = OrderId::from_u64(54321);
        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_order_id_clone() {
        let id = OrderId::from_u64(12345);
        let cloned_id = id;
        assert_eq!(id, cloned_id);
    }

    #[test]
    fn test_order_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(OrderId::from_u64(12345));
        set.insert(OrderId::from_u64(54321));
        assert!(set.contains(&OrderId::from_u64(12345)));
        assert!(set.contains(&OrderId::from_u64(54321)));
        assert!(!set.contains(&OrderId::from_u64(99999)));
        set.insert(OrderId::from_u64(12345));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_serialize_deserialize() {
        let id = OrderId::from_u64(12345);
        let serialized = serde_json::to_string(&id).unwrap();
        let expected_uuid = id.to_string();
        assert!(serialized.contains(&expected_uuid));

        let deserialized: OrderId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, id);
    }

    #[test]
    fn test_from_str_valid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let order_id = OrderId::from_str(uuid_str).unwrap();
        assert_eq!(order_id.to_string(), uuid_str);

        // Test that legacy conversions still work through string format
        let u64_id = 12345;
        let order_id_from_u64 = OrderId::from_u64(u64_id);
        let order_id_str = order_id_from_u64.to_string();
        let order_id_parsed = OrderId::from_str(&order_id_str).unwrap();
        assert_eq!(order_id_from_u64, order_id_parsed);
    }

    #[test]
    fn test_from_str_invalid() {
        assert!(OrderId::from_str("").is_err());
        assert!(OrderId::from_str("not-a-uuid").is_err());
        assert!(OrderId::from_str("550e8400-e29b-41d4-a716").is_err()); // Incomplete UUID
    }

    #[test]
    fn test_display() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let id = OrderId::from_uuid(uuid);
        assert_eq!(format!("{id}"), "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn test_roundtrip() {
        // Test U64 round trip
        let original = 12345u64;
        let id = OrderId::from_u64(original);
        let string = id.to_string();
        let parsed = string.parse::<OrderId>().unwrap();
        assert_eq!(parsed, id);

        // Test UUID round trip
        let uuid = Uuid::new_v4();
        let id = OrderId::from_uuid(uuid);
        let string = id.to_string();
        let parsed = string.parse::<OrderId>().unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn test_side_opposite() {
        // Test the opposite method on Side enum
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite(), Side::Buy);

        // Double check opposite of opposite returns original
        assert_eq!(Side::Buy.opposite().opposite(), Side::Buy);
        assert_eq!(Side::Sell.opposite().opposite(), Side::Sell);
    }

    #[test]
    fn test_order_id_nil() {
        // Test the OrderId::nil() functionality
        let nil_id = OrderId::nil();

        // Verify it's equal to the OrderId::Uuid(Uuid::nil())
        assert_eq!(nil_id, OrderId::Uuid(Uuid::nil()));

        // Convert to string and verify
        let str_representation = nil_id.to_string();
        assert_eq!(str_representation, "00000000-0000-0000-0000-000000000000");

        // Parse from string and verify roundtrip
        let parsed = OrderId::from_str(&str_representation).unwrap();
        assert_eq!(parsed, nil_id);
    }

    #[test]
    fn test_order_id_default() {
        let default_id = OrderId::default();
        assert_ne!(default_id, OrderId::nil());

        // The default implementation calls new(), which creates a random ULID
        // So we just need to verify it's not nil
        assert_ne!(default_id, OrderId::Uuid(Uuid::nil()));
    }
}
