use crate::Error;
use std::convert::TryFrom;

/// Defines fundamental types that can be associated with a device.
/// Drivers set the type for each device they manage and, for devices
/// that can be set, only accept values of the correct type.

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// For devices that return/accept a simple true/false, on/off,
    /// etc., state.
    Bool(bool),

    /// For devices that return/accept an integer value. It is stored
    /// as a signed, 64-bit value so a device returning an unsinged,
    /// 32-bit integer will have enough space to represent it.
    Int(i64),

    /// For devices that return/accept floating point numbers.
    Flt(f64),

    /// For devices that return/accept text. Since strings can greatly
    /// vary in size, care must be taken when returning this type. A
    /// driver that returns strings rapidly should keep them short.
    /// Longer strings should be returned at a slower rate. If the
    /// system takes too much time serializing string data, it could
    /// throw other portions of DrMem out of "soft real-time".
    Str(String),

    /// Represents a color value.
    Rgba(u32),
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

impl TryFrom<Value> for i64 {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Int(v) = value {
	    Ok(v)
        } else {
	    Err(Error::TypeError)
        }
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Int(value)
    }
}

impl TryFrom<Value> for i32 {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Int(v) = value {
	    if let Ok(v) = i32::try_from(v) {
                return Ok(v);
	    }
        }
        Err(Error::TypeError)
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Value::Int(i64::from(value))
    }
}

impl TryFrom<Value> for u32 {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Int(v) = value {
	    if let Ok(v) = u32::try_from(v) {
                return Ok(v);
	    }
        }
        Err(Error::TypeError)
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Value::Int(i64::from(value))
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
        Value::Int(i64::from(value))
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
        Value::Int(i64::from(value))
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


#[cfg(test)]
mod tests {
    use std::convert::TryFrom;
    use super::*;

    #[test]
    fn test_device_values_from() {
        assert_eq!(Value::Bool(true), Value::from(true));
        assert_eq!(Value::Bool(false), Value::from(false));

        assert_eq!(Value::Int(0), Value::from(0i64));
        assert_eq!(Value::Int(-1), Value::from(-1i32));
        assert_eq!(Value::Int(2), Value::from(2u32));
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

        // Check that we can convert i64 values.

        assert!(i64::try_from(Value::Bool(true)).is_err());
        assert_eq!(i64::try_from(Value::Int(0)), Ok(0i64));
        assert!(i64::try_from(Value::Flt(0.0)).is_err());
        assert!(i64::try_from(Value::Str(String::from("hello"))).is_err());

        // Check that we can convert u32 values.

        assert!(i32::try_from(Value::Bool(true)).is_err());
        assert_eq!(i32::try_from(Value::Int(0x7fffffffi64)), Ok(0x7fffffffi32));
        assert_eq!(
            i32::try_from(Value::Int(-0x80000000i64)),
            Ok(-0x80000000i32)
        );
        assert!(i32::try_from(Value::Int(0x80000000i64)).is_err());
        assert!(i32::try_from(Value::Int(-0x80000000i64 - 1i64)).is_err());
        assert!(i32::try_from(Value::Flt(0.0)).is_err());
        assert!(i32::try_from(Value::Str(String::from("hello"))).is_err());

        // Check that we can convert u32 values.

        assert!(u32::try_from(Value::Bool(true)).is_err());
        assert_eq!(u32::try_from(Value::Int(0xffffffffi64)), Ok(0xffffffffu32));
        assert_eq!(u32::try_from(Value::Int(0i64)), Ok(0u32));
        assert!(u32::try_from(Value::Int(0x100000000i64)).is_err());
        assert!(u32::try_from(Value::Int(-1i64)).is_err());
        assert!(u32::try_from(Value::Flt(0.0)).is_err());
        assert!(u32::try_from(Value::Str(String::from("hello"))).is_err());

        // Check that we can convert i16 values.

        assert!(i16::try_from(Value::Bool(true)).is_err());
        assert_eq!(i16::try_from(Value::Int(0x7fffi64)), Ok(0x7fffi16));
        assert_eq!(i16::try_from(Value::Int(-0x8000i64)), Ok(-0x8000i16));
        assert!(i16::try_from(Value::Int(0x8000i64)).is_err());
        assert!(i16::try_from(Value::Int(-0x8000i64 - 1i64)).is_err());
        assert!(i16::try_from(Value::Flt(0.0)).is_err());
        assert!(i16::try_from(Value::Str(String::from("hello"))).is_err());

        // Check that we can convert u16 values.

        assert!(u16::try_from(Value::Bool(true)).is_err());
        assert_eq!(u16::try_from(Value::Int(0xffffi64)), Ok(0xffffu16));
        assert_eq!(u16::try_from(Value::Int(0i64)), Ok(0u16));
        assert!(u16::try_from(Value::Int(0x10000i64)).is_err());
        assert!(u16::try_from(Value::Int(-1i64)).is_err());
        assert!(u16::try_from(Value::Flt(0.0)).is_err());
        assert!(u16::try_from(Value::Str(String::from("hello"))).is_err());
    }
}
