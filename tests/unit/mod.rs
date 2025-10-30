/******************************************************************************
   Author: Joaquín Béjar García
   Email: jb@taunais.com
   Date: 28/3/25
******************************************************************************/

use pricelevel::{OrderCommon, OrderId, Order, PriceLevel, Side, TimeInForce};

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_standard_order_creation() {
        let order = Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(1),
                price: 10000,
                display_quantity: 100,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };

        assert_eq!(order.id(), OrderId::from_u64(1));
        assert_eq!(order.price(), 10000);
        assert_eq!(order.display_quantity(), 100);
        assert_eq!(order.side(), Side::Buy);
        assert_eq!(order.timestamp(), 1616823000000);
        assert_eq!(order.time_in_force(), TimeInForce::Gtc);
    }

    #[test]
    fn test_iceberg_order_creation() {
        let order = Order::<()>::IcebergOrder {
            common: OrderCommon {
                id: OrderId::from_u64(2),
                price: 10000,
                display_quantity: 50,
                side: Side::Sell,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
            reserve_quantity: 150,
        };

        assert_eq!(order.id(), OrderId::from_u64(2));
        assert_eq!(order.price(), 10000);
        assert_eq!(order.display_quantity(), 50);
        assert_eq!(order.reserve_quantity(), 150);
        assert_eq!(order.side(), Side::Sell);
    }

    #[test]
    fn test_price_level_basic_operations() {
        let price_level = PriceLevel::new(10000);

        let order = Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(100),
                price: 10000,
                display_quantity: 75,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };

        price_level.add_order(order);

        assert_eq!(price_level.price(), 10000);
        assert_eq!(price_level.display_quantity(), 75);
        assert_eq!(price_level.order_count(), 1);
    }

    #[test]
    fn test_order_type_display() {
        let order = Order::<()>::PostOnly {
            common: OrderCommon {
                id: OrderId::from_u64(3),
                price: 9950,
                display_quantity: 25,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Gtc,
                extra_fields: (),
            },
        };

        let display_str = format!("{}", order);
        assert!(display_str.contains("PostOnly"));
        assert!(display_str.contains("9950"));
        assert!(display_str.contains("25"));
    }

    #[test]
    fn test_order_time_properties() {
        let order = Order::<()>::Standard {
            common: OrderCommon {
                id: OrderId::from_u64(4),
                price: 10050,
                display_quantity: 200,
                side: Side::Buy,
                timestamp: 1616823000000,
                time_in_force: TimeInForce::Ioc,
                extra_fields: (),
            },
        };

        assert!(order.is_immediate());
        assert!(!order.is_fill_or_kill());
        assert!(!order.is_post_only());
    }
}
