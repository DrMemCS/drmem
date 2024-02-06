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

    /// For devices that render color values.
    Color(palette::LinSrgb<u8>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(v) => write!(f, "{}", v),
            Value::Int(v) => write!(f, "{}", v),
            Value::Flt(v) => write!(f, "{}", v),
            Value::Str(v) => write!(f, "\"{}\"", v),
            Value::Color(v) => {
                write!(f, "\"#{:02x}{:02x}{:02x}\"", v.red, v.green, v.blue)
            }
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

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Str(value.into())
    }
}

impl From<palette::LinSrgb<u8>> for Value {
    fn from(value: palette::LinSrgb<u8>) -> Self {
        Value::Color(value)
    }
}

impl TryFrom<Value> for palette::LinSrgb<u8> {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Value::Color(v) = value {
            Ok(v)
        } else {
            Err(Error::TypeError)
        }
    }
}

// Parses a color from a string. The only form currently supported is
// "#RRGGBB" where the red, green, and blue portions are two hex
// digits. Even though this function takes a slice, it's a private
// function and we know we only call it when the slice has exactly 6
// hex digits so we don't have to test to see if the result exceeds
// 0xffffff.

fn parse_color(s: &[u8]) -> Option<Value> {
    let mut result = 0u32;

    for ii in s {
        if ii.is_ascii_digit() {
            result = (result << 4) + (ii - b'0') as u32;
        } else if (b'A'..=b'F').contains(ii) {
            result = (result << 4) + (ii - b'A' + 10) as u32;
        } else if (b'a'..=b'f').contains(ii) {
            result = (result << 4) + (ii - b'a' + 10) as u32;
        } else {
            return None;
        }
    }

    Some(Value::Color(palette::LinSrgb::new(
        (result >> 16) as u8,
        (result >> 8) as u8,
        result as u8,
    )))
}

impl TryFrom<&toml::value::Value> for Value {
    type Error = Error;

    fn try_from(value: &toml::value::Value) -> Result<Self, Self::Error> {
        match value {
            toml::value::Value::Boolean(v) => Ok(Value::Bool(*v)),
            toml::value::Value::Integer(v) => i32::try_from(*v)
                .map(Value::Int)
                .map_err(|_| Error::TypeError),
            toml::value::Value::Float(v) => Ok(Value::Flt(*v)),
            toml::value::Value::String(v) => match v.as_bytes() {
                tmp @ &[b'#', _, _, _, _, _, _] => {
                    if let Some(v) = parse_color(&tmp[1..]) {
                        Ok(v)
                    } else {
                        Ok(Value::Str(v.clone()))
                    }
                }
                _ => Ok(Value::Str(v.clone())),
            },
            _ => Err(Error::TypeError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn test_device_values_to() {
        assert_eq!("false", format!("{}", Value::Bool(false)));
        assert_eq!("true", format!("{}", Value::Bool(true)));

        assert_eq!("0", format!("{}", Value::Int(0)));
        assert_eq!("1", format!("{}", Value::Int(1)));
        assert_eq!("-1", format!("{}", Value::Int(-1)));
        assert_eq!("-2147483648", format!("{}", Value::Int(-0x80000000)));
        assert_eq!("2147483647", format!("{}", Value::Int(0x7fffffff)));

        assert_eq!(
            "\"#010203\"",
            format!("{}", Value::Color(palette::LinSrgb::new(1, 2, 3)))
        );
    }

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

        assert_eq!(Value::Str(String::from("hello")), Value::from("hello"));

        // Cycle through 256 values.

        for ii in 1..=255u8 {
            let r: u8 = ii;
            let g: u8 = ii ^ 0xa5u8;
            let b: u8 = 255u8 - ii;

            assert_eq!(
                Value::Color(palette::LinSrgb::new(r, g, b)),
                Value::from(palette::LinSrgb::new(r, g, b))
            );
        }
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

    #[test]
    fn test_toml_value_tryfrom() {
        assert_eq!(
            Value::try_from(&toml::value::Value::Boolean(true)),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::Boolean(false)),
            Ok(Value::Bool(false))
        );

        assert_eq!(
            Value::try_from(&toml::value::Value::Integer(0)),
            Ok(Value::Int(0))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::Integer(10)),
            Ok(Value::Int(10))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::Integer(-10)),
            Ok(Value::Int(-10))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::Integer(0x7fffffff)),
            Ok(Value::Int(0x7fffffff))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::Integer(-0x80000000)),
            Ok(Value::Int(-0x80000000))
        );
        assert!(
            Value::try_from(&toml::value::Value::Integer(-0x80000001)).is_err(),
        );
        assert!(
            Value::try_from(&toml::value::Value::Integer(0x80000000)).is_err(),
        );

        assert_eq!(
            Value::try_from(&toml::value::Value::Float(0.0)),
            Ok(Value::Flt(0.0))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::Float(10.0)),
            Ok(Value::Flt(10.0))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::Float(-10.0)),
            Ok(Value::Flt(-10.0))
        );

        assert_eq!(
            Value::try_from(&toml::value::Value::String("hello".into())),
            Ok(Value::Str("hello".into()))
        );

        assert_eq!(
            Value::try_from(&toml::value::Value::String("#".into())),
            Ok(Value::Str("#".into()))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::String("#1".into())),
            Ok(Value::Str("#1".into()))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::String("#12".into())),
            Ok(Value::Str("#12".into()))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::String("#123".into())),
            Ok(Value::Str("#123".into()))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::String("#1234".into())),
            Ok(Value::Str("#1234".into()))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::String("#12345".into())),
            Ok(Value::Str("#12345".into()))
        );
        assert_eq!(
            Value::try_from(&toml::value::Value::String("#1234567".into())),
            Ok(Value::Str("#1234567".into()))
        );

        // Cycle through 256 semi-random colors. Make sure the parsing
        // handles upper and lower case hex digits.

        for ii in 1..=255u8 {
            let r: u8 = ii;
            let g: u8 = ii ^ 0xa5u8;
            let b: u8 = 255u8 - ii;

            assert_eq!(
                Value::try_from(&toml::value::Value::String(format!(
                    "#{:02x}{:02x}{:02x}",
                    r, g, b
                )))
                .unwrap(),
                Value::Color(palette::LinSrgb::new(r, g, b))
            );
            assert_eq!(
                Value::try_from(&toml::value::Value::String(format!(
                    "#{:02X}{:02X}{:02X}",
                    r, g, b
                )))
                .unwrap(),
                Value::Color(palette::LinSrgb::new(r, g, b))
            );
        }

        assert!(Value::try_from(&toml::value::Value::Datetime(
            toml::value::Datetime {
                date: None,
                time: None,
                offset: None
            }
        ))
        .is_err());
        assert!(Value::try_from(&toml::value::Value::Array(vec![])).is_err());
        assert!(Value::try_from(&toml::value::Value::Table(
            toml::map::Map::new()
        ))
        .is_err());
    }
}
