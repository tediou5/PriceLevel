use crate::errors::PriceLevelError;
use crate::order::base::{OrderId, Side};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Represents a request to update an existing order
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum OrderUpdate {
    /// Update the price of an order
    UpdatePrice {
        /// ID of the order to update
        order_id: OrderId,
        /// New price for the order
        new_price: u64,
    },

    /// Update the quantity of an order
    UpdateQuantity {
        /// ID of the order to update
        order_id: OrderId,
        /// New quantity for the order
        new_quantity: u64,
    },

    /// Update both price and quantity of an order
    UpdatePriceAndQuantity {
        /// ID of the order to update
        order_id: OrderId,
        /// New price for the order
        new_price: u64,
        /// New quantity for the order
        new_quantity: u64,
    },

    /// Cancel an order
    Cancel {
        /// ID of the order to cancel
        order_id: OrderId,
    },

    /// Replace an order entirely with a new one
    Replace {
        /// ID of the order to replace
        order_id: OrderId,
        /// New price for the replacement order
        price: u64,
        /// New quantity for the replacement order
        quantity: u64,
        /// Side of the market (unchanged)
        side: Side,
    },
}

impl FromStr for OrderUpdate {
    type Err = PriceLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err(PriceLevelError::InvalidFormat(
                "Invalid order update format".to_string(),
            ));
        }

        let update_type = parts[0];
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

        // Parse order_id field which is common to all update types
        let order_id_str = get_field("order_id")?;
        let order_id =
            OrderId::from_str(order_id_str).map_err(|_| PriceLevelError::InvalidFieldValue {
                field: "order_id".to_string(),
                value: order_id_str.to_string(),
            })?;

        match update_type {
            "UpdatePrice" => {
                let new_price_str = get_field("new_price")?;
                let new_price = parse_u64("new_price", new_price_str)?;

                Ok(OrderUpdate::UpdatePrice {
                    order_id,
                    new_price,
                })
            }
            "UpdateQuantity" => {
                let new_quantity_str = get_field("new_quantity")?;
                let new_quantity = parse_u64("new_quantity", new_quantity_str)?;

                Ok(OrderUpdate::UpdateQuantity {
                    order_id,
                    new_quantity,
                })
            }
            "UpdatePriceAndQuantity" => {
                let new_price_str = get_field("new_price")?;
                let new_price = parse_u64("new_price", new_price_str)?;

                let new_quantity_str = get_field("new_quantity")?;
                let new_quantity = parse_u64("new_quantity", new_quantity_str)?;

                Ok(OrderUpdate::UpdatePriceAndQuantity {
                    order_id,
                    new_price,
                    new_quantity,
                })
            }
            "Cancel" => Ok(OrderUpdate::Cancel { order_id }),
            "Replace" => {
                let price_str = get_field("price")?;
                let price = parse_u64("price", price_str)?;

                let quantity_str = get_field("quantity")?;
                let quantity = parse_u64("quantity", quantity_str)?;

                let side_str = get_field("side")?;
                let side =
                    Side::from_str(side_str).map_err(|_| PriceLevelError::InvalidFieldValue {
                        field: "side".to_string(),
                        value: side_str.to_string(),
                    })?;

                Ok(OrderUpdate::Replace {
                    order_id,
                    price,
                    quantity,
                    side,
                })
            }
            _ => Err(PriceLevelError::UnknownOrderType(update_type.to_string())),
        }
    }
}

impl std::fmt::Display for OrderUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderUpdate::UpdatePrice {
                order_id,
                new_price,
            } => {
                write!(f, "UpdatePrice:order_id={order_id};new_price={new_price}")
            }
            OrderUpdate::UpdateQuantity {
                order_id,
                new_quantity,
            } => {
                write!(
                    f,
                    "UpdateQuantity:order_id={order_id};new_quantity={new_quantity}"
                )
            }
            OrderUpdate::UpdatePriceAndQuantity {
                order_id,
                new_price,
                new_quantity,
            } => {
                write!(
                    f,
                    "UpdatePriceAndQuantity:order_id={order_id};new_price={new_price};new_quantity={new_quantity}"
                )
            }
            OrderUpdate::Cancel { order_id } => {
                write!(f, "Cancel:order_id={order_id}")
            }
            OrderUpdate::Replace {
                order_id,
                price,
                quantity,
                side,
            } => {
                write!(
                    f,
                    "Replace:order_id={order_id};price={price};quantity={quantity};side={side}"
                )
            }
        }
    }
}

#[cfg(test)]
mod tests_order_update {
    use crate::errors::PriceLevelError;
    use crate::order::base::{OrderId, Side};
    use crate::order::update::OrderUpdate;
    use std::str::FromStr;

    #[test]
    fn test_update_price_from_str() {
        let input = "UpdatePrice:order_id=00000000-0000-007b-0000-000000000000;new_price=1000";
        let result = OrderUpdate::from_str(input).unwrap();

        match result {
            OrderUpdate::UpdatePrice {
                order_id,
                new_price,
            } => {
                assert_eq!(order_id, OrderId::from_u64(123));
                assert_eq!(new_price, 1000);
            }
            _ => panic!("Expected UpdatePrice variant"),
        }
    }

    #[test]
    fn test_update_quantity_from_str() {
        let input = "UpdateQuantity:order_id=00000000-0000-01c8-0000-000000000000;new_quantity=50";
        let result = OrderUpdate::from_str(input).unwrap();
        match result {
            OrderUpdate::UpdateQuantity {
                order_id,
                new_quantity,
            } => {
                assert_eq!(order_id, OrderId::from_u64(456));
                assert_eq!(new_quantity, 50);
            }
            _ => panic!("Expected UpdateQuantity variant"),
        }
    }

    #[test]
    fn test_update_price_and_quantity_from_str() {
        let input = "UpdatePriceAndQuantity:order_id=00000000-0000-0315-0000-000000000000;new_price=2000;new_quantity=30";
        let result = OrderUpdate::from_str(input).unwrap();
        match result {
            OrderUpdate::UpdatePriceAndQuantity {
                order_id,
                new_price,
                new_quantity,
            } => {
                assert_eq!(order_id, OrderId::from_u64(789));
                assert_eq!(new_price, 2000);
                assert_eq!(new_quantity, 30);
            }
            _ => panic!("Expected UpdatePriceAndQuantity variant"),
        }
    }

    #[test]
    fn test_cancel_from_str() {
        let input = "Cancel:order_id=00000000-0000-0065-0000-000000000000";
        let result = OrderUpdate::from_str(input).unwrap();

        match result {
            OrderUpdate::Cancel { order_id } => {
                assert_eq!(order_id, OrderId::from_u64(101));
            }
            _ => panic!("Expected Cancel variant"),
        }
    }

    #[test]
    fn test_replace_from_str() {
        let input =
            "Replace:order_id=00000000-0000-00ca-0000-000000000000;price=3000;quantity=40;side=BUY";
        let result = OrderUpdate::from_str(input).unwrap();

        match result {
            OrderUpdate::Replace {
                order_id,
                price,
                quantity,
                side,
            } => {
                assert_eq!(order_id, OrderId::from_u64(202));
                assert_eq!(price, 3000);
                assert_eq!(quantity, 40);
                assert_eq!(side, Side::Buy);
            }
            _ => panic!("Expected Replace variant"),
        }
    }

    #[test]
    fn test_replace_with_sell_side_from_str() {
        let input = "Replace:order_id=00000000-0000-012f-0000-000000000000;price=4000;quantity=60;side=SELL";
        let result = OrderUpdate::from_str(input).unwrap();

        match result {
            OrderUpdate::Replace {
                order_id,
                price,
                quantity,
                side,
            } => {
                assert_eq!(order_id, OrderId::from_u64(303));
                assert_eq!(price, 4000);
                assert_eq!(quantity, 60);
                assert_eq!(side, Side::Sell);
            }
            _ => panic!("Expected Replace variant"),
        }
    }

    #[test]
    fn test_invalid_format() {
        let input = "UpdatePrice;order_id=123;new_price=1000";
        let result = OrderUpdate::from_str(input);

        assert!(result.is_err());
        match result.unwrap_err() {
            PriceLevelError::InvalidFormat(_) => {}
            err => panic!("Expected InvalidFormat error, got {err:?}"),
        }
    }

    #[test]
    fn test_unknown_order_type() {
        let input = "Unknown:order_id=00000000-0000-007b-0000-000000000000;new_price=1000";
        let result = OrderUpdate::from_str(input);

        assert!(result.is_err());
        match result.unwrap_err() {
            PriceLevelError::UnknownOrderType(type_name) => {
                assert_eq!(type_name, "Unknown");
            }
            err => panic!("Expected UnknownOrderType error, got {err:?}"),
        }
    }

    #[test]
    fn test_missing_field() {
        let input = "UpdatePrice:order_id=00000000-0000-007b-0000-000000000000"; // missing new_price
        let result = OrderUpdate::from_str(input);

        assert!(result.is_err());
        match result.unwrap_err() {
            PriceLevelError::MissingField(field) => {
                assert_eq!(field, "new_price");
            }
            err => panic!("Expected MissingField error, got {err:?}"),
        }
    }

    #[test]
    fn test_invalid_field_value() {
        let input = "UpdatePrice:order_id=abc;new_price=1000"; // invalid order_id
        let result = OrderUpdate::from_str(input);

        assert!(result.is_err());
        match result.unwrap_err() {
            PriceLevelError::InvalidFieldValue { field, value } => {
                assert_eq!(field, "order_id");
                assert_eq!(value, "abc");
            }
            err => panic!("Expected InvalidFieldValue error, got {err:?}"),
        }
    }

    #[test]
    fn test_display_update_price() {
        let update = OrderUpdate::UpdatePrice {
            order_id: OrderId::from_u64(123),
            new_price: 1000,
        };

        assert_eq!(
            update.to_string(),
            "UpdatePrice:order_id=00000000-0000-007b-0000-000000000000;new_price=1000"
        );
    }

    #[test]
    fn test_display_update_quantity() {
        let update = OrderUpdate::UpdateQuantity {
            order_id: OrderId::from_u64(456),
            new_quantity: 50,
        };

        assert_eq!(
            update.to_string(),
            "UpdateQuantity:order_id=00000000-0000-01c8-0000-000000000000;new_quantity=50"
        );
    }

    #[test]
    fn test_display_update_price_and_quantity() {
        let update = OrderUpdate::UpdatePriceAndQuantity {
            order_id: OrderId::from_u64(789),
            new_price: 2000,
            new_quantity: 30,
        };

        assert_eq!(
            update.to_string(),
            "UpdatePriceAndQuantity:order_id=00000000-0000-0315-0000-000000000000;new_price=2000;new_quantity=30"
        );
    }

    #[test]
    fn test_display_cancel() {
        let update = OrderUpdate::Cancel {
            order_id: OrderId::from_u64(101),
        };

        assert_eq!(
            update.to_string(),
            "Cancel:order_id=00000000-0000-0065-0000-000000000000"
        );
    }

    #[test]
    fn test_display_replace() {
        let update = OrderUpdate::Replace {
            order_id: OrderId::from_u64(202),
            price: 3000,
            quantity: 40,
            side: Side::Buy,
        };

        assert_eq!(
            update.to_string(),
            "Replace:order_id=00000000-0000-00ca-0000-000000000000;price=3000;quantity=40;side=BUY"
        );
    }

    #[test]
    fn test_roundtrip_parsing() {
        // Create instances of each variant
        let updates = vec![
            OrderUpdate::UpdatePrice {
                order_id: OrderId::from_u64(123),
                new_price: 1000,
            },
            OrderUpdate::UpdateQuantity {
                order_id: OrderId::from_u64(456),
                new_quantity: 50,
            },
            OrderUpdate::UpdatePriceAndQuantity {
                order_id: OrderId::from_u64(789),
                new_price: 2000,
                new_quantity: 30,
            },
            OrderUpdate::Cancel {
                order_id: OrderId::from_u64(101),
            },
            OrderUpdate::Replace {
                order_id: OrderId::from_u64(202),
                price: 3000,
                quantity: 40,
                side: Side::Buy,
            },
            OrderUpdate::Replace {
                order_id: OrderId::from_u64(303),
                price: 4000,
                quantity: 60,
                side: Side::Sell,
            },
        ];

        // Test round-trip for each variant
        for update in updates {
            let string_representation = update.to_string();
            let parsed_update = OrderUpdate::from_str(&string_representation).unwrap();

            // Compare the debug representation since OrderUpdate doesn't implement PartialEq
            assert_eq!(format!("{update:?}"), format!("{:?}", parsed_update));
        }
    }

    #[test]
    fn test_order_update_display_detailed() {
        // Test display of UpdatePrice
        let update = OrderUpdate::UpdatePrice {
            order_id: OrderId::from_u64(123),
            new_price: 10500,
        };
        let display_string = update.to_string();
        assert_eq!(
            display_string,
            "UpdatePrice:order_id=00000000-0000-007b-0000-000000000000;new_price=10500"
        );

        // Test display of UpdateQuantity
        let update = OrderUpdate::UpdateQuantity {
            order_id: OrderId::from_u64(456),
            new_quantity: 75,
        };
        let display_string = update.to_string();
        assert_eq!(
            display_string,
            "UpdateQuantity:order_id=00000000-0000-01c8-0000-000000000000;new_quantity=75"
        );

        // Test display of UpdatePriceAndQuantity
        let update = OrderUpdate::UpdatePriceAndQuantity {
            order_id: OrderId::from_u64(789),
            new_price: 11000,
            new_quantity: 50,
        };
        let display_string = update.to_string();
        assert_eq!(
            display_string,
            "UpdatePriceAndQuantity:order_id=00000000-0000-0315-0000-000000000000;new_price=11000;new_quantity=50"
        );

        // Test display of Replace
        let update = OrderUpdate::Replace {
            order_id: OrderId::from_u64(202),
            price: 12000,
            quantity: 60,
            side: Side::Sell,
        };
        let display_string = update.to_string();
        assert_eq!(
            display_string,
            "Replace:order_id=00000000-0000-00ca-0000-000000000000;price=12000;quantity=60;side=SELL"
        );
    }

    #[test]
    fn test_order_update_from_str_replace_side() {
        // Test parsing of Replace with Buy side
        let input = "Replace:order_id=00000000-0000-00ca-0000-000000000000;price=12000;quantity=60;side=BUY";
        let update = OrderUpdate::from_str(input).unwrap();

        match update {
            OrderUpdate::Replace {
                order_id,
                price,
                quantity,
                side,
            } => {
                assert_eq!(order_id, OrderId::from_u64(202));
                assert_eq!(price, 12000);
                assert_eq!(quantity, 60);
                assert_eq!(side, Side::Buy);
            }
            _ => panic!("Expected Replace variant"),
        }

        // Test parsing of Replace with Sell side
        let input = "Replace:order_id=00000000-0000-00ca-0000-000000000000;price=12000;quantity=60;side=SELL";
        let update = OrderUpdate::from_str(input).unwrap();

        match update {
            OrderUpdate::Replace {
                order_id,
                price,
                quantity,
                side,
            } => {
                assert_eq!(order_id, OrderId::from_u64(202));
                assert_eq!(price, 12000);
                assert_eq!(quantity, 60);
                assert_eq!(side, Side::Sell);
            }
            _ => panic!("Expected Replace variant"),
        }

        // Test parsing with invalid side (should fail)
        let input = "Replace:order_id=00000000-0000-00ca-0000-000000000000;price=12000;quantity=60;side=INVALID";
        let result = OrderUpdate::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_display_cancel() {
        let update = OrderUpdate::Cancel {
            order_id: OrderId::from_u64(123),
        };

        assert_eq!(
            update.to_string(),
            "Cancel:order_id=00000000-0000-007b-0000-000000000000"
        );
    }
}
