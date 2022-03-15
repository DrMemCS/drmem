// Copyright (c) 2022, Richard M Neswold, Jr.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
// 1. Redistributions of source code must retain the above copyright
//    notice, this list of conditions and the following disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright
//    notice, this list of conditions and the following disclaimer in the
//    documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its
//    contributors may be used to endorse or promote products derived
//    from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

///!This module defines types related to devices.

mod value {
    use crate::Error;
    use std::convert::TryFrom;

    /// Defines fundamental types that can be associated with a
    /// device. Drivers set the type for each device they manage and,
    /// for devices that can be set, only accept values of the correct
    /// type.

    #[derive(Clone, Debug, PartialEq)]
    pub enum Value {
	/// For devices that return/accept a simple true/false,
	/// on/off, etc., state.
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
}

pub use value::Value;

mod name {
    use std::fmt;
    use std::str::FromStr;
    use regex::Regex;
    use lazy_static::lazy_static;
    use crate::Error;

    /// Holds a validated device name. A device name consists of a
    /// path and a name where each portion of the name is separated
    /// with a colon. Each segment of the path or the name is composed
    /// of alpha- numeric and the dash characters. The dash cannot be
    /// the first or last character, however.
    ///
    /// More formally:
    ///
    /// ```ignore
    /// DEVICE-NAME = PATH NAME
    /// PATH = (SEGMENT ':')+
    /// NAME = SEGMENT
    /// SEGMENT = [0-9a-zA-Z] ( [0-9a-zA-Z-]* [0-9a-zA-Z] )?
    /// ```
    ///
    /// All device names will have a path and a name. Although
    /// superficially similar, device names are not like file system
    /// names. Specifically, there's no concept of moving up or down
    /// paths. The paths are to help organize device.

    #[derive(Debug, PartialEq)]
    pub struct Name {
	path: String,
	name: String,
    }

    impl Name {
	/// Creates an instance of `Name`, if the provided string
	/// describes a well-formed device name.

	pub fn create(s: &str) -> Result<Name, Error> {
            lazy_static! {
		// This regular expression parses a device name. It
		// uses the "named grouping" feature to easily tag the
		// matching sections.
		//
		// The first section matches any leading path:
		//
		//    (?P<path>(?:[\d[[:alpha:]]](?:[\d[[:alpha:]]-]*[\d[[:alpha:]]])?:)+)
		//
		// which can be written more clearly as
		//
		//    ALNUM = [0-9a-zA-Z]
		//    SEGMENT = ALNUM ((ALNUM | '-')* ALNUM)?
		//
		//    path = (SEGMENT ':')+
		//
		// The difference being that [[:alpha:]] recognizes
		// Unicode letters instead of just the ASCII "a-zA-Z"
		// letters.
		//
		// The second section represents the base name of the
		// device:
		//
		//    (?P<name>[\d[[:alpha:]]](?:[\d[[:alpha:]]-]*[\d[[:alpha:]]])?)
		//
		// which is just SEGMENT from above.

		static ref RE: Regex = Regex::new(r"^(?P<path>(?:[\d[[:alpha:]]](?:[\d[[:alpha:]]-]*[\d[[:alpha:]]])?:)+)(?P<name>[\d[[:alpha:]]](?:[\d[[:alpha:]]-]*[\d[[:alpha:]]])?)$").unwrap();
            }

            // The Regex expression is anchored to the start and end
            // of the string and both halves to which we're matching
            // are not optional. So if it returns `Some()`, we have
            // "path" and "name" entries.

            if let Some(caps) = RE.captures(s) {
		Ok(Name {
                    path: String::from(&caps["path"]),
                    name: String::from(&caps["name"]),
		})
            } else {
		Err(Error::InvArgument("invalid device path/name"))
            }
	}

	/// Returns the path of the device name without the trailing
	/// ':'.

	pub fn get_path(&self) -> &str {
            let len = self.path.len();

            &self.path[..len - 1]
	}

	/// Returns the base name of the device.

	pub fn get_name(&self) -> &str {
            &self.name
	}
    }

    impl fmt::Display for Name {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}{}", &self.path, &self.name)
	}
    }

    impl FromStr for Name {
	type Err = Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
            Name::create(s)
	}
    }

    #[cfg(test)]
    mod tests {
	use super::*;

	#[test]
	fn test_device_name() {
            assert!("".parse::<Name>().is_err());
            assert!(":".parse::<Name>().is_err());
            assert!("a".parse::<Name>().is_err());
            assert!(":a".parse::<Name>().is_err());
            assert!("a:".parse::<Name>().is_err());
            assert!("a::a".parse::<Name>().is_err());

            assert!("p:a.".parse::<Name>().is_err());
            assert!("p:a.a".parse::<Name>().is_err());
            assert!("p.a:a".parse::<Name>().is_err());
            assert!("p:a-".parse::<Name>().is_err());
            assert!("p:-a".parse::<Name>().is_err());
            assert!("p-:a".parse::<Name>().is_err());
            assert!("-p:a".parse::<Name>().is_err());

            assert_eq!(
		"p:abc".parse::<Name>().unwrap(),
		Name {
                    path: String::from("p:"),
                    name: String::from("abc"),
		}
            );
            assert_eq!(
		"p:abc1".parse::<Name>().unwrap(),
		Name {
                    path: String::from("p:"),
                    name: String::from("abc1"),
		}
            );
            assert_eq!(
		"p:abc-1".parse::<Name>().unwrap(),
		Name {
                    path: String::from("p:"),
                    name: String::from("abc-1"),
		}
            );
            assert_eq!(
		"p-1:p-2:abc".parse::<Name>().unwrap(),
		Name {
                    path: String::from("p-1:p-2:"),
                    name: String::from("abc"),
		}
            );

            let dn = "p-1:p-2:abc".parse::<Name>().unwrap();

            assert_eq!(dn.get_path(), "p-1:p-2");
            assert_eq!(dn.get_name(), "abc");

            assert_eq!(format!("{}", dn), "p-1:p-2:abc");
	}
    }
}

pub use name::Name;

mod item {
    use super::*;
    use crate::Error;
    use lazy_static::lazy_static;
    use regex::Regex;
    use std::str::FromStr;

    #[derive(Debug, PartialEq, Clone, Copy)]
    pub enum Field {
	Value,
	Unit,
	Location,
	Summary,
	Detail,
    }

    /// Holds a validated and canonicalized device specification. A full
    /// device specification is made up with the device name along with
    /// the field name of interest. If the field name is missing in the
    /// input, the canonical expansion will include the "value" field.

    #[derive(Debug, PartialEq)]
    pub struct Item {
	device: Name,
	field: Field,
    }

    impl Item {
	fn xlat_field(s: &str) -> Result<Field, Error> {
            match s {
		"value" => Ok(Field::Value),
		"unit" => Ok(Field::Unit),
		"detail" => Ok(Field::Detail),
		"summary" => Ok(Field::Summary),
		"location" => Ok(Field::Location),
		_ => Err(Error::InvArgument("invalid field name")),
            }
	}

	/// Creates an instance of `Item` if the provided string describes
	/// a well-formed device specification.

	pub fn create(s: &str) -> Result<Item, Error> {
            lazy_static! {
		// This regular expression parses a full device
		// specification. It uses the "named grouping" feature to
		// tag the matching sections.
		//
		// The first part is the device name. The regular
		// expression matches all the characters before the '.'
		// which get passed to the `Name` parser.
		//
		// The second section is the optional field name of the
		// device:
		//
		//    (?:\.(?P<field>[[:alpha:]]+))?
		//
		// It looks for a leading '.' before capturing the field
		// name itself.

		static ref RE: Regex = Regex::new(
		    r"^(?P<dev_name>[^\.]+)(?:\.(?P<field>[[:alpha:]]+))?$"
		).unwrap();
            }

            if let Some(caps) = RE.captures(s) {
		if let Some(dev_name) = caps.name("dev_name") {
                    let dev_name = dev_name.as_str().parse::<Name>()?;
                    let field = caps.name("field").map_or("value", |m| m.as_str());

                    return Ok(Item {
			device: dev_name,
			field: Item::xlat_field(field)?,
                    });
		}
            }
            Err(Error::InvArgument("invalid device specification"))
	}

	/// Returns the portion of the specification containing the path
	/// and base name of the device.

	pub fn get_device_name(&self) -> &Name {
            &self.device
	}

	/// Returns the field specified by the `Item`.

	pub fn get_field(&self) -> Field {
            self.field
	}
    }

    impl FromStr for Item {
	type Err = Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
            Item::create(s)
	}
    }

    #[cfg(test)]
    mod tests {
	use super::*;

	#[test]
	fn test_device_item() {
            assert!("p:".parse::<Item>().is_err());
            assert!("p:.".parse::<Item>().is_err());
            assert!("p:.a".parse::<Item>().is_err());
            assert!("p:a.".parse::<Item>().is_err());
            assert!("p:a.123".parse::<Item>().is_err());
            assert!("p:a.a.a".parse::<Item>().is_err());

            let v = "p-1:device".parse::<Item>().unwrap();

            assert_eq!(v.get_device_name().to_string(), "p-1:device");
            assert_eq!(v.get_field(), Field::Value);

            let v = "p:device.unit".parse::<Item>().unwrap();

            assert_eq!(v.get_device_name().to_string(), "p:device");
            assert_eq!(v.get_field(), Field::Unit);

            let v = "path:device.value".parse::<Item>().unwrap();

            assert_eq!(v.get_device_name().to_string(), "path:device");
            assert_eq!(v.get_field(), Field::Value);

            let v = "long:path:device.unit".parse::<Item>().unwrap();

            assert_eq!(v.get_device_name().to_string(), "long:path:device");
            assert_eq!(v.get_field(), Field::Unit);

            let v = "long:path:device.detail".parse::<Item>().unwrap();

            assert_eq!(v.get_device_name().to_string(), "long:path:device");
            assert_eq!(v.get_field(), Field::Detail);

            let v = "long:path:device.location".parse::<Item>().unwrap();

            assert_eq!(v.get_device_name().to_string(), "long:path:device");
            assert_eq!(v.get_field(), Field::Location);

            let v = "long:path:device.summary".parse::<Item>().unwrap();

            assert_eq!(v.get_device_name().to_string(), "long:path:device");
            assert_eq!(v.get_field(), Field::Summary);

            let v = "p:Device-123".parse::<Item>().unwrap();

            assert_eq!(v.get_device_name().to_string(), "p:Device-123");
            assert_eq!(v.get_field(), Field::Value);
	}
    }
}

pub use item::Item;

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
