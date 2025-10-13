use crate::errors::PriceLevelError;
use crate::execution::list::TransactionList;
use crate::execution::transaction::Transaction;
use crate::order::OrderId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Represents the result of a matching operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    /// The ID of the incoming order that initiated the match
    pub order_id: OrderId,

    /// List of transactions that resulted from the match
    pub transactions: TransactionList,

    /// Remaining quantity of the incoming order after matching
    pub remaining_quantity: u64,

    /// Whether the order was completely filled
    pub is_complete: bool,

    /// Any orders that were completely filled and removed from the book
    pub filled_order_ids: Vec<OrderId>,
}

impl MatchResult {
    /// Create a new empty match result
    pub fn new(order_id: OrderId, initial_quantity: u64) -> Self {
        Self {
            order_id,
            transactions: TransactionList::new(),
            remaining_quantity: initial_quantity,
            is_complete: false,
            filled_order_ids: Vec::new(),
        }
    }

    /// Add a transaction to this match result
    pub fn add_transaction(&mut self, transaction: Transaction) {
        self.remaining_quantity = self.remaining_quantity.saturating_sub(transaction.quantity);
        self.is_complete = self.remaining_quantity == 0;
        self.transactions.add(transaction);
    }

    /// Add a filled order ID to track orders removed from the book
    pub fn add_filled_order_id(&mut self, order_id: OrderId) {
        self.filled_order_ids.push(order_id);
    }

    /// Get the total executed quantity
    pub fn executed_quantity(&self) -> u64 {
        self.transactions.as_vec().iter().map(|t| t.quantity).sum()
    }

    /// Get the total value executed
    pub fn executed_value(&self) -> u64 {
        self.transactions
            .as_vec()
            .iter()
            .map(|t| t.price * t.quantity)
            .sum()
    }

    /// Calculate the average execution price
    pub fn average_price(&self) -> Option<f64> {
        let executed_qty = self.executed_quantity();
        if executed_qty == 0 {
            None
        } else {
            Some(self.executed_value() as f64 / executed_qty as f64)
        }
    }
}

impl fmt::Display for MatchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MatchResult:order_id={};remaining_quantity={};is_complete={}",
            self.order_id, self.remaining_quantity, self.is_complete
        )?;
        write!(f, ";transactions={}", self.transactions)?;
        write!(f, ";filled_order_ids=[")?;
        for (i, order_id) in self.filled_order_ids.iter().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, "{order_id}")?;
        }
        write!(f, "]")
    }
}

impl FromStr for MatchResult {
    type Err = PriceLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        fn find_next_field(s: &str, start_pos: usize) -> Result<(&str, usize), PriceLevelError> {
            let mut pos = start_pos;

            while pos < s.len() {
                if s[pos..].starts_with(';') {
                    let value = &s[start_pos..pos];
                    return Ok((value, pos + 1));
                }
                pos += 1;
            }

            if pos == s.len() {
                let value = &s[start_pos..pos];
                return Ok((value, pos));
            }

            Err(PriceLevelError::InvalidFormat)
        }
        if !s.starts_with("MatchResult:") {
            return Err(PriceLevelError::InvalidFormat);
        }

        let mut order_id_str = None;
        let mut remaining_quantity_str = None;
        let mut is_complete_str = None;
        let mut transactions_str = None;
        let mut filled_order_ids_str = None;

        let mut pos = "MatchResult:".len();

        while pos < s.len() {
            let field_end = match s[pos..].find('=') {
                Some(idx) => pos + idx,
                None => return Err(PriceLevelError::InvalidFormat),
            };

            let field_name = &s[pos..field_end];
            pos = field_end + 1;
            match field_name {
                "order_id" => {
                    let (value, next_pos) = find_next_field(s, pos)?;
                    order_id_str = Some(value);
                    pos = next_pos;
                }
                "remaining_quantity" => {
                    let (value, next_pos) = find_next_field(s, pos)?;
                    remaining_quantity_str = Some(value);
                    pos = next_pos;
                }
                "is_complete" => {
                    let (value, next_pos) = find_next_field(s, pos)?;
                    is_complete_str = Some(value);
                    pos = next_pos;
                }
                "transactions" => {
                    if !s[pos..].starts_with("Transactions:[") {
                        return Err(PriceLevelError::InvalidFormat);
                    }

                    let mut bracket_depth = 1;
                    let mut i = pos + "Transactions:[".len();

                    while i < s.len() && bracket_depth > 0 {
                        if s[i..].starts_with(']') {
                            bracket_depth -= 1;
                            if bracket_depth == 0 {
                                break;
                            }
                            i += 1;
                        } else if s[i..].starts_with('[') {
                            bracket_depth += 1;
                            i += 1;
                        } else {
                            i += 1;
                        }
                    }

                    if bracket_depth > 0 {
                        return Err(PriceLevelError::InvalidFormat);
                    }

                    transactions_str = Some(&s[pos..=i]);
                    pos = i + 1;
                    if pos < s.len() && s[pos..].starts_with(';') {
                        pos += 1;
                    } else if pos < s.len() {
                        return Err(PriceLevelError::InvalidFormat);
                    }
                }
                "filled_order_ids" => {
                    if !s[pos..].starts_with('[') {
                        return Err(PriceLevelError::InvalidFormat);
                    }

                    let mut bracket_depth = 1;
                    let mut i = pos + 1;

                    while i < s.len() && bracket_depth > 0 {
                        if s[i..].starts_with(']') {
                            bracket_depth -= 1;
                            if bracket_depth == 0 {
                                break;
                            }
                            i += 1;
                        } else if s[i..].starts_with('[') {
                            bracket_depth += 1;
                            i += 1;
                        } else {
                            i += 1;
                        }
                    }

                    if bracket_depth > 0 {
                        return Err(PriceLevelError::InvalidFormat);
                    }

                    filled_order_ids_str = Some(&s[pos..=i]);

                    pos = i + 1;
                    if pos < s.len() && s[pos..].starts_with(';') {
                        pos += 1;
                    }
                }
                _ => return Err(PriceLevelError::InvalidFormat),
            }
        }

        let order_id_str =
            order_id_str.ok_or_else(|| PriceLevelError::MissingField("order_id".to_string()))?;
        let remaining_quantity_str = remaining_quantity_str
            .ok_or_else(|| PriceLevelError::MissingField("remaining_quantity".to_string()))?;
        let is_complete_str = is_complete_str
            .ok_or_else(|| PriceLevelError::MissingField("is_complete".to_string()))?;
        let transactions_str = transactions_str
            .ok_or_else(|| PriceLevelError::MissingField("transactions".to_string()))?;
        let filled_order_ids_str = filled_order_ids_str
            .ok_or_else(|| PriceLevelError::MissingField("filled_order_ids".to_string()))?;

        let order_id =
            OrderId::from_str(order_id_str).map_err(|_| PriceLevelError::InvalidFieldValue {
                field: "order_id".to_string(),
                value: order_id_str.to_string(),
            })?;

        let remaining_quantity = remaining_quantity_str.parse::<u64>().map_err(|_| {
            PriceLevelError::InvalidFieldValue {
                field: "remaining_quantity".to_string(),
                value: remaining_quantity_str.to_string(),
            }
        })?;

        let is_complete =
            is_complete_str
                .parse::<bool>()
                .map_err(|_| PriceLevelError::InvalidFieldValue {
                    field: "is_complete".to_string(),
                    value: is_complete_str.to_string(),
                })?;

        let transactions = TransactionList::from_str(transactions_str)?;

        let filled_order_ids = if filled_order_ids_str == "[]" {
            Vec::new()
        } else {
            let content = &filled_order_ids_str[1..filled_order_ids_str.len() - 1];

            if content.is_empty() {
                Vec::new()
            } else {
                content
                    .split(',')
                    .map(|id_str| {
                        OrderId::from_str(id_str).map_err(|_| PriceLevelError::InvalidFieldValue {
                            field: "filled_order_ids".to_string(),
                            value: id_str.to_string(),
                        })
                    })
                    .collect::<Result<Vec<OrderId>, PriceLevelError>>()?
            }
        };

        Ok(MatchResult {
            order_id,
            transactions,
            remaining_quantity,
            is_complete,
            filled_order_ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::execution::list::TransactionList;
    use crate::execution::match_result::MatchResult;
    use crate::execution::transaction::Transaction;
    use crate::order::OrderId;
    use crate::order::Side;
    use std::str::FromStr;
    use tracing::info;
    use uuid::Uuid;

    // Helper function to create a test transaction
    fn create_test_transaction(
        id: Uuid,
        taker_id: u64,
        maker_id: u64,
        price: u64,
        quantity: u64,
    ) -> Transaction {
        Transaction {
            transaction_id: id,
            taker_order_id: OrderId::from_u64(taker_id),
            maker_order_id: OrderId::from_u64(maker_id),
            price,
            quantity,
            taker_side: Side::Buy,
            timestamp: 1616823000000, // + id, // Create unique timestamps
        }
    }

    #[test]
    fn test_match_result_new() {
        let result = MatchResult::new(OrderId::from_u64(123), 100);

        assert_eq!(result.order_id, OrderId::from_u64(123));
        assert_eq!(result.remaining_quantity, 100);
        assert!(!result.is_complete);
        assert!(result.transactions.is_empty());
        assert!(result.filled_order_ids.is_empty());
    }

    #[test]
    fn test_add_transaction() {
        let mut result = MatchResult::new(OrderId::from_u64(123), 100);

        // Add a transaction for 30 quantity
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction1 = create_test_transaction(uuid, 123, 456, 1000, 30);
        result.add_transaction(transaction1);

        assert_eq!(result.remaining_quantity, 70); // 100 - 30
        assert!(!result.is_complete);
        assert_eq!(result.transactions.len(), 1);

        // Add another transaction that will complete the match
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction2 = create_test_transaction(uuid, 123, 789, 1000, 70);
        result.add_transaction(transaction2);

        assert_eq!(result.remaining_quantity, 0);
        assert!(result.is_complete);
        assert_eq!(result.transactions.len(), 2);

        // Add a transaction that would exceed the remaining quantity
        // This is normally prevented by validation logic elsewhere, but testing the method
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        let transaction3 = create_test_transaction(uuid, 123, 101, 1000, 20);
        result.add_transaction(transaction3);

        // Should remain at 0 due to saturating_sub
        assert_eq!(result.remaining_quantity, 0);
        assert!(result.is_complete);
        assert_eq!(result.transactions.len(), 3);
    }

    #[test]
    fn test_add_filled_order_id() {
        let mut result = MatchResult::new(OrderId::from_u64(123), 100);

        result.add_filled_order_id(OrderId::from_u64(456));
        result.add_filled_order_id(OrderId::from_u64(789));

        assert_eq!(result.filled_order_ids.len(), 2);
        assert_eq!(result.filled_order_ids[0], OrderId::from_u64(456));
        assert_eq!(result.filled_order_ids[1], OrderId::from_u64(789));
    }

    #[test]
    fn test_executed_quantity() {
        let mut result = MatchResult::new(OrderId::from_u64(123), 100);

        // No transactions yet
        assert_eq!(result.executed_quantity(), 0);

        // Add some transactions
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        result.add_transaction(create_test_transaction(uuid, 123, 456, 1000, 30));
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        result.add_transaction(create_test_transaction(uuid, 123, 789, 1000, 20));

        assert_eq!(result.executed_quantity(), 50); // 30 + 20
    }

    #[test]
    fn test_executed_value() {
        let mut result = MatchResult::new(OrderId::from_u64(123), 100);

        // No transactions yet
        assert_eq!(result.executed_value(), 0);

        // Add transactions with different prices
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        result.add_transaction(create_test_transaction(uuid, 123, 456, 1000, 30)); // Value: 30,000
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        result.add_transaction(create_test_transaction(uuid, 123, 789, 1200, 20)); // Value: 24,000

        assert_eq!(result.executed_value(), 54000); // 30,000 + 24,000
    }

    #[test]
    fn test_average_price() {
        let mut result = MatchResult::new(OrderId::from_u64(123), 100);

        // No transactions yet
        assert_eq!(result.average_price(), None);

        // Add transactions with different prices
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        result.add_transaction(create_test_transaction(uuid, 123, 456, 1000, 30)); // Value: 30,000
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        result.add_transaction(create_test_transaction(uuid, 123, 789, 1200, 20)); // Value: 24,000

        // Average price: 54,000 / 50 = 1,080
        assert_eq!(result.average_price(), Some(1080.0));
    }

    #[test]
    fn test_display() {
        let mut result = MatchResult::new(OrderId::from_u64(123), 100);

        // Test display with empty transactions and filled_order_ids
        let display_str = result.to_string();

        assert!(
            display_str
                .starts_with("MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=100;is_complete=false")
        );
        assert!(display_str.contains("transactions=Transactions:[]"));
        assert!(display_str.contains("filled_order_ids=[]"));

        // Add some transactions and filled order IDs
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        result.add_transaction(create_test_transaction(uuid, 123, 456, 1000, 30));
        result.add_filled_order_id(OrderId::from_u64(456));

        let display_str = result.to_string();
        assert!(
            display_str
                .starts_with("MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=70;is_complete=false")
        );
        assert!(
            display_str.contains("Transaction:transaction_id=6ba7b810-9dad-11d1-80b4-00c04fd430c8")
        );
        assert!(display_str.contains("filled_order_ids=[00000000-0000-01c8-0000-000000000000]"));
    }

    #[test]
    fn test_from_str_valid() {
        let input = "MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=70;is_complete=false;transactions=Transactions:[];filled_order_ids=[]";
        let result = match MatchResult::from_str(input) {
            Ok(r) => r,
            Err(e) => {
                panic!("Test failed: {e:?}");
            }
        };

        assert_eq!(result.order_id, OrderId::from_u64(123));
        assert_eq!(result.remaining_quantity, 70);
        assert!(!result.is_complete);
        assert!(result.transactions.is_empty());
        assert!(result.filled_order_ids.is_empty());

        // Test parsing with transactions and filled order IDs
        let input = "MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=70;is_complete=false;transactions=Transactions:[Transaction:transaction_id=6ba7b810-9dad-11d1-80b4-00c04fd430c8;taker_order_id=00000000-0000-007b-0000-000000000000;maker_order_id=00000000-0000-01c8-0000-000000000000;price=1000;quantity=30;taker_side=BUY;timestamp=1616823000001];filled_order_ids=[00000000-0000-01c8-0000-000000000000]";
        let result = MatchResult::from_str(input).unwrap();

        assert_eq!(result.order_id, OrderId::from_u64(123));
        assert_eq!(result.remaining_quantity, 70);
        assert!(!result.is_complete);
        assert_eq!(result.transactions.len(), 1);
        assert_eq!(result.filled_order_ids.len(), 1);
        assert_eq!(result.filled_order_ids[0], OrderId::from_u64(456));
    }

    #[test]
    fn test_from_str_invalid_format() {
        // Test invalid prefix
        let input = "InvalidPrefix:order_id=123;remaining_quantity=70;is_complete=false;transactions=Transactions:[];filled_order_ids=[]";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());

        // Test missing field
        let input =
            "MatchResult:order_id=123;remaining_quantity=70;is_complete=false;filled_order_ids=[]";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());

        // Test invalid value type
        let input = "MatchResult:order_id=abc;remaining_quantity=70;is_complete=false;transactions=Transactions:[];filled_order_ids=[]";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());

        // Test invalid boolean
        let input = "MatchResult:order_id=123;remaining_quantity=70;is_complete=invalidbool;transactions=Transactions:[];filled_order_ids=[]";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());

        // Test invalid filled_order_ids format
        let input = "MatchResult:order_id=123;remaining_quantity=70;is_complete=false;transactions=Transactions:[];filled_order_ids=invalid";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip() {
        // Create a match result with some data
        let mut original = MatchResult::new(OrderId::from_u64(123), 100);
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        original.add_transaction(create_test_transaction(uuid, 123, 456, 1000, 30));
        let uuid = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
        original.add_transaction(create_test_transaction(uuid, 123, 789, 1200, 20));
        original.add_filled_order_id(OrderId::from_u64(456));
        original.add_filled_order_id(OrderId::from_u64(789));

        // Convert to string
        let string_representation = original.to_string();
        info!("String generate: '{}'", string_representation);

        // Parse back
        let parsed = match MatchResult::from_str(&string_representation) {
            Ok(r) => r,
            Err(e) => {
                panic!("Test failed: {e:?}");
            }
        };

        // Verify all fields match
        assert_eq!(parsed.order_id, original.order_id);
        assert_eq!(parsed.remaining_quantity, original.remaining_quantity);
        assert_eq!(parsed.is_complete, original.is_complete);
        assert_eq!(parsed.filled_order_ids, original.filled_order_ids);

        // Verify transactions (need to check each one since Transaction might not implement PartialEq)
        assert_eq!(parsed.transactions.len(), original.transactions.len());
        for (i, transaction) in original.transactions.as_vec().iter().enumerate() {
            let parsed_transaction = &parsed.transactions.as_vec()[i];
            assert_eq!(
                parsed_transaction.transaction_id,
                transaction.transaction_id
            );
            assert_eq!(
                parsed_transaction.taker_order_id,
                transaction.taker_order_id
            );
            assert_eq!(
                parsed_transaction.maker_order_id,
                transaction.maker_order_id
            );
            assert_eq!(parsed_transaction.price, transaction.price);
            assert_eq!(parsed_transaction.quantity, transaction.quantity);
            assert_eq!(parsed_transaction.taker_side, transaction.taker_side);
            assert_eq!(parsed_transaction.timestamp, transaction.timestamp);
        }
    }

    #[test]
    fn test_with_multiple_filled_order_ids() {
        // Create a match result with multiple filled order IDs
        let mut result = MatchResult::new(OrderId::from_u64(123), 100); // 00000000-0000-007b-0000-000000000000
        result.add_filled_order_id(OrderId::from_u64(456)); // 00000000-0000-01c8-0000-000000000000
        result.add_filled_order_id(OrderId::from_u64(789)); // 00000000-0000-0315-0000-000000000000
        result.add_filled_order_id(OrderId::from_u64(101)); // 00000000-0000-0065-0000-000000000000

        // Convert to string
        let string_representation = result.to_string();

        // Verify filled_order_ids format
        assert!(string_representation.contains("filled_order_ids=[00000000-0000-01c8-0000-000000000000,00000000-0000-0315-0000-000000000000,00000000-0000-0065-0000-000000000000]"));

        // Parse back
        let parsed = MatchResult::from_str(&string_representation).unwrap();

        // Verify filled_order_ids were parsed correctly
        assert_eq!(parsed.filled_order_ids.len(), 3);
        assert_eq!(parsed.filled_order_ids[0], OrderId::from_u64(456));
        assert_eq!(parsed.filled_order_ids[1], OrderId::from_u64(789));
        assert_eq!(parsed.filled_order_ids[2], OrderId::from_u64(101));
    }

    #[test]
    fn test_with_empty_transactions_and_filled_ids() {
        // Test with explicitly empty collections
        let mut result = MatchResult::new(OrderId::from_u64(123), 100);
        result.transactions = TransactionList::new(); // Explicitly empty
        result.filled_order_ids = Vec::new(); // Explicitly empty

        // Convert to string
        let string_representation = result.to_string();

        // Parse back
        let parsed = MatchResult::from_str(&string_representation).unwrap();

        // Verify
        assert!(parsed.transactions.is_empty());
        assert!(parsed.filled_order_ids.is_empty());
    }

    #[test]
    fn test_match_result_from_str_parsing_edge_cases() {
        // Test parsing a complete match result with all fields
        let input = "MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=70;is_complete=false;transactions=Transactions:[Transaction:transaction_id=6ba7b810-9dad-11d1-80b4-00c04fd430c8;taker_order_id=00000000-0000-007b-0000-000000000000;maker_order_id=00000000-0000-01c8-0000-000000000000;price=1000;quantity=30;taker_side=BUY;timestamp=1616823000000];filled_order_ids=[00000000-0000-01c8-0000-000000000000,00000000-0000-0315-0000-000000000000]";

        let result = MatchResult::from_str(input).unwrap();

        assert_eq!(result.order_id, OrderId::from_u64(123));
        assert_eq!(result.remaining_quantity, 70);
        assert!(!result.is_complete);
        assert_eq!(result.transactions.len(), 1);
        assert_eq!(result.filled_order_ids.len(), 2);
        assert_eq!(result.filled_order_ids[0], OrderId::from_u64(456));
        assert_eq!(result.filled_order_ids[1], OrderId::from_u64(789));

        // Test parsing with complex nested structures
        let input = "MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=70;is_complete=false;transactions=Transactions:[Transaction:transaction_id=6ba7b810-9dad-11d1-80b4-00c04fd430c8;taker_order_id=00000000-0000-007b-0000-000000000000;maker_order_id=00000000-0000-01c8-0000-000000000000;price=1000;quantity=30;taker_side=BUY;timestamp=1616823000000,Transaction:transaction_id=7ca7b810-9dad-11d1-80b4-00c04fd430c8;taker_order_id=00000000-0000-007b-0000-000000000000;maker_order_id=00000000-0000-0315-0000-000000000000;price=1100;quantity=40;taker_side=BUY;timestamp=1616823000001];filled_order_ids=[00000000-0000-01c8-0000-000000000000,00000000-0000-0315-0000-000000000000]";

        let result = MatchResult::from_str(input).unwrap();

        assert_eq!(result.transactions.len(), 2);
        let transaction1 = &result.transactions.as_vec()[0];
        let transaction2 = &result.transactions.as_vec()[1];

        assert_eq!(transaction1.quantity, 30);
        assert_eq!(transaction2.quantity, 40);
    }

    #[test]
    fn test_match_result_parsing_error_cases() {
        // Test invalid field_name
        let input = "MatchResult:invalid_field=value;remaining_quantity=70;is_complete=false;transactions=Transactions:[];filled_order_ids=[]";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());

        // Test bracket mismatch in transactions
        let input = "MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=70;is_complete=false;transactions=Transactions:[Transaction:transaction_id=6ba7b810-9dad-11d1-80b4-00c04fd430c8;taker_order_id=00000000-0000-007b-0000-000000000000;filled_order_ids=[]";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());

        // Test invalid transactions format
        let input = "MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=70;is_complete=false;transactions=NotTransactions:[];filled_order_ids=[]";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());

        // Test invalid filled_order_ids format
        let input = "MatchResult:order_id=00000000-0000-007b-0000-000000000000;remaining_quantity=70;is_complete=false;transactions=Transactions:[];filled_order_ids=NotAnArray";
        let result = MatchResult::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_match_result_find_fields() {
        // Create a match result with simple field structure
        let mut result = MatchResult::new(OrderId::from_u64(123), 100);
        result.remaining_quantity = 50;
        result.is_complete = false;

        // Convert to string
        let string_representation = result.to_string();

        // Manually parse some fields to test the find_next_field function
        let order_id_pos = string_representation.find("order_id=").unwrap() + "order_id=".len();
        let semicolon_pos = string_representation[order_id_pos..].find(';').unwrap() + order_id_pos;
        let order_id_str = &string_representation[order_id_pos..semicolon_pos];

        assert_eq!(order_id_str, "00000000-0000-007b-0000-000000000000");

        let remaining_pos = string_representation.find("remaining_quantity=").unwrap()
            + "remaining_quantity=".len();
        let semicolon_pos =
            string_representation[remaining_pos..].find(';').unwrap() + remaining_pos;
        let remaining_str = &string_representation[remaining_pos..semicolon_pos];

        assert_eq!(remaining_str, "50");

        let complete_pos =
            string_representation.find("is_complete=").unwrap() + "is_complete=".len();
        let semicolon_pos = string_representation[complete_pos..].find(';').unwrap() + complete_pos;
        let complete_str = &string_representation[complete_pos..semicolon_pos];

        assert_eq!(complete_str, "false");
    }
}
