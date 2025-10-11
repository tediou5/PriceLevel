use std::fmt::{Debug, Display, Formatter, Result};

/// Represents errors that can occur when processing price levels in trading operations.
///
/// This enum encapsulates various error conditions that might arise during order book
/// management, price validation, and other trading-related operations.
///
/// # Examples
///
/// ```
/// use pricelevel::PriceLevelError;
///
/// // Creating a parse error
/// let error = PriceLevelError::ParseError {
///     message: "Failed to parse price: invalid decimal format".to_string()
/// };
///
/// // Creating a missing field error
/// let missing_field_error = PriceLevelError::MissingField("price".to_string());
/// ```
pub enum PriceLevelError {
    /// Error that occurs when parsing fails with a specific message.
    ///
    /// This variant is used when string conversion or data parsing operations fail.
    ParseError {
        /// Descriptive message explaining the parsing failure
        message: String,
    },

    /// Error indicating that the input is in an invalid format.
    ///
    /// This is a general error for when the input data doesn't conform to expected patterns
    /// but doesn't fit into more specific error categories.
    InvalidFormat,

    /// Error indicating an unrecognized order type was provided.
    ///
    /// Used when the system encounters an order type string that isn't in the supported set.
    /// The string parameter contains the unrecognized order type.
    UnknownOrderType(String),

    /// Error indicating a required field is missing.
    ///
    /// Used when a mandatory field is absent in the input data.
    /// The string parameter specifies which field is missing.
    MissingField(String),

    /// Error indicating a field has an invalid value.
    ///
    /// This error occurs when a field's value is present but doesn't meet validation criteria.
    InvalidFieldValue {
        /// The name of the field with the invalid value
        field: String,
        /// The invalid value as a string representation
        value: String,
    },

    /// Error indicating an operation cannot be performed for the specified reason.
    ///
    /// Used when an action is prevented due to business rules or system constraints.
    InvalidOperation {
        /// Explanation of why the operation is invalid
        message: String,
    },

    /// Error raised when serialization of internal data structures fails.
    SerializationError {
        /// Descriptive message with the serialization failure details
        message: String,
    },

    /// Error raised when deserialization of external data into internal structures fails.
    DeserializationError {
        /// Descriptive message with the deserialization failure details
        message: String,
    },

    /// Error raised when a checksum validation fails while restoring a snapshot.
    ChecksumMismatch {
        /// The checksum that was expected according to the serialized payload
        expected: String,
        /// The checksum that was computed from the provided payload
        actual: String,
    },
}
impl Display for PriceLevelError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            PriceLevelError::ParseError { message } => write!(f, "{message}"),
            PriceLevelError::InvalidFormat => write!(f, "Invalid format"),
            PriceLevelError::UnknownOrderType(order_type) => {
                write!(f, "Unknown order type: {order_type}")
            }
            PriceLevelError::MissingField(field) => write!(f, "Missing field: {field}"),
            PriceLevelError::InvalidFieldValue { field, value } => {
                write!(f, "Invalid value for field {field}: {value}")
            }
            PriceLevelError::InvalidOperation { message } => {
                write!(f, "Invalid operation: {message}")
            }
            PriceLevelError::SerializationError { message } => {
                write!(f, "Serialization error: {message}")
            }
            PriceLevelError::DeserializationError { message } => {
                write!(f, "Deserialization error: {message}")
            }
            PriceLevelError::ChecksumMismatch { expected, actual } => {
                write!(f, "Checksum mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

impl Debug for PriceLevelError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            PriceLevelError::ParseError { message } => write!(f, "{message}"),
            PriceLevelError::InvalidFormat => write!(f, "Invalid format"),
            PriceLevelError::UnknownOrderType(order_type) => {
                write!(f, "Unknown order type: {order_type}")
            }
            PriceLevelError::MissingField(field) => write!(f, "Missing field: {field}"),
            PriceLevelError::InvalidFieldValue { field, value } => {
                write!(f, "Invalid value for field {field}: {value}")
            }
            PriceLevelError::InvalidOperation { message } => {
                write!(f, "Invalid operation: {message}")
            }
            PriceLevelError::SerializationError { message } => {
                write!(f, "Serialization error: {message}")
            }
            PriceLevelError::DeserializationError { message } => {
                write!(f, "Deserialization error: {message}")
            }
            PriceLevelError::ChecksumMismatch { expected, actual } => {
                write!(f, "Checksum mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for PriceLevelError {}

#[cfg(test)]
mod tests {
    use crate::errors::PriceLevelError;
    use std::error::Error;

    #[test]
    fn test_parse_error_display() {
        let error = PriceLevelError::ParseError {
            message: "Failed to parse".to_string(),
        };
        assert_eq!(error.to_string(), "Failed to parse");
    }

    #[test]
    fn test_invalid_format_display() {
        let error = PriceLevelError::InvalidFormat;
        assert_eq!(error.to_string(), "Invalid format");
    }

    #[test]
    fn test_unknown_order_type_display() {
        let error = PriceLevelError::UnknownOrderType("CustomOrder".to_string());
        assert_eq!(error.to_string(), "Unknown order type: CustomOrder");
    }

    #[test]
    fn test_missing_field_display() {
        let error = PriceLevelError::MissingField("price".to_string());
        assert_eq!(error.to_string(), "Missing field: price");
    }

    #[test]
    fn test_invalid_field_value_display() {
        let error = PriceLevelError::InvalidFieldValue {
            field: "quantity".to_string(),
            value: "abc".to_string(),
        };
        assert_eq!(error.to_string(), "Invalid value for field quantity: abc");
    }

    #[test]
    fn test_invalid_operation_display() {
        let error = PriceLevelError::InvalidOperation {
            message: "Cannot update price to same value".to_string(),
        };
        assert_eq!(
            error.to_string(),
            "Invalid operation: Cannot update price to same value"
        );
    }

    #[test]
    fn test_debug_implementation() {
        // Test that Debug produces the same output as Display for our cases

        let errors = [
            PriceLevelError::ParseError {
                message: "Debug test".to_string(),
            },
            PriceLevelError::InvalidFormat,
            PriceLevelError::UnknownOrderType("TestOrder".to_string()),
            PriceLevelError::MissingField("id".to_string()),
            PriceLevelError::InvalidFieldValue {
                field: "side".to_string(),
                value: "MIDDLE".to_string(),
            },
            PriceLevelError::InvalidOperation {
                message: "Debug operation test".to_string(),
            },
        ];

        for error in &errors {
            assert_eq!(format!("{error:?}"), error.to_string());
        }
    }

    #[test]
    fn test_implements_error_trait() {
        // Test that our error type implements the standard Error trait
        let error = PriceLevelError::InvalidFormat;
        let _: &dyn Error = &error;

        // If this compiles, the test passes since it confirms
        // PriceLevelError implements the Error trait
    }

    #[test]
    fn test_error_source() {
        // Test that source() returns None as we don't have nested errors
        let error = PriceLevelError::InvalidFormat;
        assert!(error.source().is_none());
    }

    #[test]
    fn test_clone_and_compare_errors() {
        // Test error equality (though it's not derived in the original code)
        // We'll test by comparing string representation
        let error1 = PriceLevelError::MissingField("price".to_string());
        let error2 = PriceLevelError::MissingField("price".to_string());
        let error3 = PriceLevelError::MissingField("quantity".to_string());

        assert_eq!(error1.to_string(), error2.to_string());
        assert_ne!(error1.to_string(), error3.to_string());
    }

    #[test]
    fn test_error_formatting_consistency() {
        // Test that formatting is consistent across different error variants
        let parse_error = PriceLevelError::ParseError {
            message: "test message".to_string(),
        };
        assert_eq!(parse_error.to_string(), "test message");

        let field_error = PriceLevelError::MissingField("field".to_string());
        assert_eq!(field_error.to_string(), "Missing field: field");

        // Verify error messages don't have trailing whitespace or unexpected formatting
        for error in [
            PriceLevelError::InvalidFormat.to_string(),
            PriceLevelError::UnknownOrderType("Test".to_string()).to_string(),
            PriceLevelError::InvalidFieldValue {
                field: "f".to_string(),
                value: "v".to_string(),
            }
            .to_string(),
            PriceLevelError::InvalidOperation {
                message: "op".to_string(),
            }
            .to_string(),
        ] {
            assert_eq!(
                error.trim(),
                error,
                "Error message contains unexpected whitespace"
            );
        }
    }
}
