use crate::types::Error;
use std::{convert::TryFrom, fmt};

/// Defines fundamental types that can be associated with a device.
/// Drivers set the type for each device they manage and, for devices
/// that can be set, only accept values of the correct type.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// For devices that return/accept a simple true/false, on/off,
    /// etc., state.
    Bool(bool),

    /// For devices that return/accept an integer value. It is stored
    /// as a signed, 32-bit. This type should primarily be used for
    /// digital inputs/outputs. There is no 64-bit version because
    /// Javascript doesn't support a 64-bit integer. For integer
    /// values greater than 32 bits, use a `Flt` since it can
    /// losslessly handle integers up to 52 bits.
    Int(i32),

    /// For devices that return/accept floating point numbers or
    /// integers up to 52 bits.
    Flt(f64),

    /// For devices that return/accept text. Since strings can greatly
    /// vary in size, care must be taken when returning this type. A
    /// driver that returns strings rapidly should keep them short.
    /// Longer strings should be returned at a slower rate. If the
    /// system takes too much time serializing string data, it could
    /// throw other portions of DrMem out of "soft real-time".
    Str(String),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(v) => write!(f, "{}", v),
            Value::Int(v) => write!(f, "{}", v),
            Value::Flt(v) => write!(f, "{}", v),
            Value::Str(v) => write!(f, "\"{}\"", v),
        }
    }
}

impl TryFrom<Value> for bool {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Bool(v) = value {
            Ok(v)
        } else {
            Err(Error::TypeError)
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl TryFrom<Value> for i32 {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Int(v) = value {
            return Ok(v);
        }
        Err(Error::TypeError)
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Value::Int(value)
    }
}

impl TryFrom<Value> for i16 {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Int(v) = value {
            if let Ok(v) = i16::try_from(v) {
                return Ok(v);
            }
        }
        Err(Error::TypeError)
    }
}

impl From<i16> for Value {
    fn from(value: i16) -> Self {
        Value::Int(i32::from(value))
    }
}

impl TryFrom<Value> for u16 {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Int(v) = value {
            if let Ok(v) = u16::try_from(v) {
                return Ok(v);
            }
        }
        Err(Error::TypeError)
    }
}

impl From<u16> for Value {
    fn from(value: u16) -> Self {
        Value::Int(i32::from(value))
    }
}

impl TryFrom<Value> for f64 {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Flt(v) = value {
            Ok(v)
        } else {
            Err(Error::TypeError)
        }
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Flt(value)
    }
}

impl TryFrom<Value> for String {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Str(v) = value {
            Ok(v)
        } else {
            Err(Error::TypeError)
        }
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::Str(value)
    }
}

impl From<&String> for Value {
    fn from(value: &String) -> Self {
        Value::Str(value.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn test_device_values_from() {
        assert_eq!(Value::Bool(true), Value::from(true));
        assert_eq!(Value::Bool(false), Value::from(false));

        assert_eq!(Value::Int(0), Value::from(0i32));
        assert_eq!(Value::Int(-1), Value::from(-1i32));
        assert_eq!(Value::Int(2), Value::from(2i32));
        assert_eq!(Value::Int(-3), Value::from(-3i16));
        assert_eq!(Value::Int(4), Value::from(4u16));

        assert_eq!(Value::Flt(5.0), Value::from(5.0f64));

        assert_eq!(
            Value::Str(String::from("hello")),
            Value::from(String::from("hello"))
        );
    }

    #[test]
    fn test_device_values_tryfrom() {
        // Check that we can convert bool values.

        assert_eq!(bool::try_from(Value::Bool(true)), Ok(true));
        assert!(bool::try_from(Value::Int(0)).is_err());
        assert!(bool::try_from(Value::Flt(0.0)).is_err());
        assert!(bool::try_from(Value::Str(String::from("hello"))).is_err());

        // Check that we can convert i32 values.

        assert!(i32::try_from(Value::Bool(true)).is_err());
        assert_eq!(i32::try_from(Value::Int(0x7fffffffi32)), Ok(0x7fffffffi32));
        assert_eq!(
            i32::try_from(Value::Int(-0x80000000i32)),
            Ok(-0x80000000i32)
        );
        assert!(i32::try_from(Value::Flt(0.0)).is_err());
        assert!(i32::try_from(Value::Str(String::from("hello"))).is_err());

        // Check that we can convert i16 values.

        assert!(i16::try_from(Value::Bool(true)).is_err());
        assert_eq!(i16::try_from(Value::Int(0x7fffi32)), Ok(0x7fffi16));
        assert_eq!(i16::try_from(Value::Int(-0x8000i32)), Ok(-0x8000i16));
        assert!(i16::try_from(Value::Int(0x8000i32)).is_err());
        assert!(i16::try_from(Value::Int(-0x8000i32 - 1i32)).is_err());
        assert!(i16::try_from(Value::Flt(0.0)).is_err());
        assert!(i16::try_from(Value::Str(String::from("hello"))).is_err());

        // Check that we can convert u16 values.

        assert!(u16::try_from(Value::Bool(true)).is_err());
        assert_eq!(u16::try_from(Value::Int(0xffffi32)), Ok(0xffffu16));
        assert_eq!(u16::try_from(Value::Int(0i32)), Ok(0u16));
        assert!(u16::try_from(Value::Int(0x10000i32)).is_err());
        assert!(u16::try_from(Value::Int(-1i32)).is_err());
        assert!(u16::try_from(Value::Flt(0.0)).is_err());
        assert!(u16::try_from(Value::Str(String::from("hello"))).is_err());
    }
}
