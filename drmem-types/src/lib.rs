// Copyright (c) 2020-2022, Richard M Neswold, Jr.
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

//! Defines fundamental types used throughout the DrMem codebase.

use lazy_static::lazy_static;
use regex::Regex;
use std::convert::{From, TryFrom};
use std::fmt;
use std::str::FromStr;

/// Enumerates all the errors that can be reported in DrMem. Authors
/// for new drivers or storage backends should try to map their
/// errors into one of these values. If no current value is
/// appropriate, a new one could be added (requiring a new release of
/// this crate) but make sure the new error code is generic enough
/// that it may be useful for other drivers or backends. For instance,
/// don't add an error value that is specific to Redis. Add a more
/// general value and use the associated description string to explain
/// the details.

#[derive(Debug, PartialEq)]
pub enum DrMemError {
    /// Returned whenever a resource cannot be found.
    NotFound,

    /// A resource is already in use.
    InUse,

    /// The device name is already registered to another driver.
    DeviceDefined(String),

    /// Reported when the peer of a communication channel has closed
    /// its handle.
    MissingPeer(String),

    /// A type mismatch is preventing the operation from continuing.
    TypeError,

    /// An invalid value was provided.
    InvArgument(&'static str),

    /// Returned when a communication error occurred with the backend
    /// database. Each backend will have its own recommendations on
    /// how to recover.
    DbCommunicationError,

    /// The requested operation cannot complete because the process
    /// hasn't provided proper authentication credentials.
    AuthenticationError,

    /// The requested operation couldn't complete. The description
    /// field will have more information for the user.
    OperationError,

    /// A bad parameter was given in a configuration or a
    /// configuration was missing a required parameter.
    BadConfig,

    /// A dependent library introduced a new error that hasn't been
    /// properly mapped in DrMem. This needs to be reported as a bug.
    UnknownError,
}

impl std::error::Error for DrMemError {}

impl fmt::Display for DrMemError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DrMemError::NotFound => write!(f, "item not found"),
            DrMemError::InUse => write!(f, "item is in use"),
            DrMemError::DeviceDefined(name) => {
                write!(f, "device {} is already defined", &name)
            }
            DrMemError::MissingPeer(detail) => {
                write!(f, "{} is missing peer", detail)
            }
            DrMemError::TypeError => write!(f, "incorrect type"),
            DrMemError::InvArgument(s) => write!(f, "{}", s),
            DrMemError::DbCommunicationError => {
                write!(f, "db communication error")
            }
            DrMemError::AuthenticationError => write!(f, "permission error"),
            DrMemError::OperationError => {
                write!(f, "couldn't complete operation")
            }
            DrMemError::BadConfig => write!(f, "bad configuration"),
            DrMemError::UnknownError => write!(f, "unhandled error"),
        }
    }
}

/// Defines fundamental types that can be associated with a
/// device. Drivers set the type for each device they manage and, for
/// devices that can be set, only accept values of the correct type.
#[derive(Clone, Debug, PartialEq)]
pub enum DeviceValue {
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

impl TryFrom<DeviceValue> for bool {
    type Error = DrMemError;

    fn try_from(value: DeviceValue) -> Result<Self, Self::Error> {
        if let DeviceValue::Bool(v) = value {
            Ok(v)
        } else {
            Err(DrMemError::TypeError)
        }
    }
}

impl From<bool> for DeviceValue {
    fn from(value: bool) -> Self {
        DeviceValue::Bool(value)
    }
}

impl TryFrom<DeviceValue> for i64 {
    type Error = DrMemError;

    fn try_from(value: DeviceValue) -> Result<Self, Self::Error> {
        if let DeviceValue::Int(v) = value {
            Ok(v)
        } else {
            Err(DrMemError::TypeError)
        }
    }
}

impl From<i64> for DeviceValue {
    fn from(value: i64) -> Self {
        DeviceValue::Int(value)
    }
}

impl TryFrom<DeviceValue> for i32 {
    type Error = DrMemError;

    fn try_from(value: DeviceValue) -> Result<Self, Self::Error> {
        if let DeviceValue::Int(v) = value {
	    if let Ok(v) = i32::try_from(v) {
		return Ok(v)
	    }
        }
        Err(DrMemError::TypeError)
    }
}

impl From<i32> for DeviceValue {
    fn from(value: i32) -> Self {
        DeviceValue::Int(i64::from(value))
    }
}

impl TryFrom<DeviceValue> for u32 {
    type Error = DrMemError;

    fn try_from(value: DeviceValue) -> Result<Self, Self::Error> {
        if let DeviceValue::Int(v) = value {
	    if let Ok(v) = u32::try_from(v) {
		return Ok(v)
	    }
        }
        Err(DrMemError::TypeError)
    }
}

impl From<u32> for DeviceValue {
    fn from(value: u32) -> Self {
        DeviceValue::Int(i64::from(value))
    }
}

impl TryFrom<DeviceValue> for i16 {
    type Error = DrMemError;

    fn try_from(value: DeviceValue) -> Result<Self, Self::Error> {
        if let DeviceValue::Int(v) = value {
	    if let Ok(v) = i16::try_from(v) {
		return Ok(v)
	    }
        }
        Err(DrMemError::TypeError)
    }
}

impl From<i16> for DeviceValue {
    fn from(value: i16) -> Self {
        DeviceValue::Int(i64::from(value))
    }
}

impl TryFrom<DeviceValue> for u16 {
    type Error = DrMemError;

    fn try_from(value: DeviceValue) -> Result<Self, Self::Error> {
        if let DeviceValue::Int(v) = value {
	    if let Ok(v) = u16::try_from(v) {
		return Ok(v)
	    }
        }
        Err(DrMemError::TypeError)
    }
}

impl From<u16> for DeviceValue {
    fn from(value: u16) -> Self {
        DeviceValue::Int(i64::from(value))
    }
}

impl TryFrom<DeviceValue> for f64 {
    type Error = DrMemError;

    fn try_from(value: DeviceValue) -> Result<Self, Self::Error> {
        if let DeviceValue::Flt(v) = value {
            Ok(v)
        } else {
            Err(DrMemError::TypeError)
        }
    }
}

impl From<f64> for DeviceValue {
    fn from(value: f64) -> Self {
        DeviceValue::Flt(value)
    }
}

impl TryFrom<DeviceValue> for String {
    type Error = DrMemError;

    fn try_from(value: DeviceValue) -> Result<Self, Self::Error> {
        if let DeviceValue::Str(v) = value {
            Ok(v)
        } else {
            Err(DrMemError::TypeError)
        }
    }
}

impl From<String> for DeviceValue {
    fn from(value: String) -> Self {
        DeviceValue::Str(value)
    }
}

/// Holds a validated device name. A device name consists of a path
/// and a name where each portion of the name is separated with a
/// colon. Each segment of the path or the name is composed of alpha-
/// numeric and the dash characters. The dash cannot be the first or
/// last character, however.
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
pub struct DeviceName {
    path: String,
    name: String,
}

impl DeviceName {
    /// Creates an instance of `DeviceName`, if the provided string
    /// describes a well-formed device name.

    pub fn create(s: &str) -> Result<DeviceName, DrMemError> {
        lazy_static! {
            // This regular expression parses a device name. It uses
            // the "named grouping" feature to easily tag the matching
            // sections.
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

	// The Regex expression is anchored to the start and end of
	// the string and both halves to which we're matching are not
	// optional. So if it returns `Some()`, we have "path" and
	// "name" entries.

        if let Some(caps) = RE.captures(s) {
            Ok(DeviceName {
                path: String::from(&caps["path"]),
                name: String::from(&caps["name"]),
            })
        } else {
            Err(DrMemError::InvArgument("invalid device path/name"))
        }
    }

    /// Returns the path of the device name without the trailing ':'.

    pub fn get_path(&self) -> &str {
	let len = self.path.len();

	&self.path[..len - 1]
    }

    /// Returns the base name of the device.

    pub fn get_name(&self) -> &str {
	&self.name
    }
}

impl fmt::Display for DeviceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", &self.path, &self.name)
    }
}

impl FromStr for DeviceName {
    type Err = DrMemError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DeviceName::create(s)
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DeviceField {
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
pub struct DeviceSpec {
    device: DeviceName,
    field: DeviceField,
}

impl DeviceSpec {
    fn xlat_field(s: &str) -> Result<DeviceField, DrMemError> {
        match s {
            "value" => Ok(DeviceField::Value),
            "unit" => Ok(DeviceField::Unit),
            "detail" => Ok(DeviceField::Detail),
            "summary" => Ok(DeviceField::Summary),
            "location" => Ok(DeviceField::Location),
            _ => Err(DrMemError::InvArgument("invalid field name")),
        }
    }

    /// Creates an instance of `DeviceSpec` if the provided string
    /// describes a well-formed device specification.

    pub fn create(s: &str) -> Result<DeviceSpec, DrMemError> {
        lazy_static! {
            // This regular expression parses a full device
            // specification. It uses the "named grouping" feature to
            // tag the matching sections.
            //
            // The first part is the device name. The regular
            // expression matches all the characters before the
            // '.' which get passed to the `DeviceName` parser.
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
                let dev_name = dev_name.as_str().parse::<DeviceName>()?;
                let field = caps.name("field").map_or("value", |m| m.as_str());

                return Ok(DeviceSpec {
                    device: dev_name,
                    field: DeviceSpec::xlat_field(field)?,
                });
            }
        }
        Err(DrMemError::InvArgument("invalid device specification"))
    }

    /// Returns the portion of the specification containing the path
    /// and base name of the device.

    pub fn get_device_name(&self) -> &DeviceName {
	&self.device
    }

    /// Returns the field specified by the `DeviceSpec`.

    pub fn get_field(&self) -> DeviceField {
	self.field
    }
}

impl FromStr for DeviceSpec {
    type Err = DrMemError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DeviceSpec::create(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_values_from() {
	assert_eq!(DeviceValue::Bool(true), DeviceValue::from(true));
	assert_eq!(DeviceValue::Bool(false), DeviceValue::from(false));

	assert_eq!(DeviceValue::Int(0), DeviceValue::from(0i64));
	assert_eq!(DeviceValue::Int(-1), DeviceValue::from(-1i32));
	assert_eq!(DeviceValue::Int(2), DeviceValue::from(2u32));
	assert_eq!(DeviceValue::Int(-3), DeviceValue::from(-3i16));
	assert_eq!(DeviceValue::Int(4), DeviceValue::from(4u16));

	assert_eq!(DeviceValue::Flt(5.0), DeviceValue::from(5.0f64));

	assert_eq!(
	    DeviceValue::Str(String::from("hello")),
	    DeviceValue::from(String::from("hello"))
	);
    }

    #[test]
    fn test_device_values_tryfrom() {

	// Check that we can convert bool values.

	assert_eq!(bool::try_from(DeviceValue::Bool(true)), Ok(true));
	assert!(bool::try_from(DeviceValue::Int(0)).is_err());
	assert!(bool::try_from(DeviceValue::Flt(0.0)).is_err());
	assert!(
	    bool::try_from(DeviceValue::Str(String::from("hello"))).is_err()
	);

	// Check that we can convert i64 values.

	assert!(i64::try_from(DeviceValue::Bool(true)).is_err());
	assert_eq!(i64::try_from(DeviceValue::Int(0)), Ok(0i64));
	assert!(i64::try_from(DeviceValue::Flt(0.0)).is_err());
	assert!(
	    i64::try_from(DeviceValue::Str(String::from("hello"))).is_err()
	);

	// Check that we can convert u32 values.

	assert!(i32::try_from(DeviceValue::Bool(true)).is_err());
	assert_eq!(
	    i32::try_from(DeviceValue::Int(0x7fffffffi64)),
	    Ok(0x7fffffffi32)
	);
	assert_eq!(
	    i32::try_from(DeviceValue::Int(-0x80000000i64)),
	    Ok(-0x80000000i32)
	);
	assert!(i32::try_from(DeviceValue::Int(0x80000000i64)).is_err());
	assert!(
	    i32::try_from(DeviceValue::Int(-0x80000000i64 - 1i64)).is_err()
	);
	assert!(i32::try_from(DeviceValue::Flt(0.0)).is_err());
	assert!(
	    i32::try_from(DeviceValue::Str(String::from("hello"))).is_err()
	);

	// Check that we can convert u32 values.

	assert!(u32::try_from(DeviceValue::Bool(true)).is_err());
	assert_eq!(
	    u32::try_from(DeviceValue::Int(0xffffffffi64)),
	    Ok(0xffffffffu32)
	);
	assert_eq!(u32::try_from(DeviceValue::Int(0i64)), Ok(0u32));
	assert!(u32::try_from(DeviceValue::Int(0x100000000i64)).is_err());
	assert!(u32::try_from(DeviceValue::Int(-1i64)).is_err());
	assert!(u32::try_from(DeviceValue::Flt(0.0)).is_err());
	assert!(
	    u32::try_from(DeviceValue::Str(String::from("hello"))).is_err()
	);

	// Check that we can convert i16 values.

	assert!(i16::try_from(DeviceValue::Bool(true)).is_err());
	assert_eq!(i16::try_from(DeviceValue::Int(0x7fffi64)), Ok(0x7fffi16));
	assert_eq!(
	    i16::try_from(DeviceValue::Int(-0x8000i64)),
	    Ok(-0x8000i16)
	);
	assert!(i16::try_from(DeviceValue::Int(0x8000i64)).is_err());
	assert!(
	    i16::try_from(DeviceValue::Int(-0x8000i64 - 1i64)).is_err()
	);
	assert!(i16::try_from(DeviceValue::Flt(0.0)).is_err());
	assert!(
	    i16::try_from(DeviceValue::Str(String::from("hello"))).is_err()
	);

	// Check that we can convert u16 values.

	assert!(u16::try_from(DeviceValue::Bool(true)).is_err());
	assert_eq!(u16::try_from(DeviceValue::Int(0xffffi64)), Ok(0xffffu16));
	assert_eq!(u16::try_from(DeviceValue::Int(0i64)), Ok(0u16));
	assert!(u16::try_from(DeviceValue::Int(0x10000i64)).is_err());
	assert!(u16::try_from(DeviceValue::Int(-1i64)).is_err());
	assert!(u16::try_from(DeviceValue::Flt(0.0)).is_err());
	assert!(
	    u16::try_from(DeviceValue::Str(String::from("hello"))).is_err()
	);
    }

    #[test]
    fn test_device_name() {
        assert!("".parse::<DeviceName>().is_err());
        assert!(":".parse::<DeviceName>().is_err());
        assert!("a".parse::<DeviceName>().is_err());
        assert!(":a".parse::<DeviceName>().is_err());
        assert!("a:".parse::<DeviceName>().is_err());
        assert!("a::a".parse::<DeviceName>().is_err());

        assert!("p:a.".parse::<DeviceName>().is_err());
        assert!("p:a.a".parse::<DeviceName>().is_err());
        assert!("p.a:a".parse::<DeviceName>().is_err());
        assert!("p:a-".parse::<DeviceName>().is_err());
        assert!("p:-a".parse::<DeviceName>().is_err());
        assert!("p-:a".parse::<DeviceName>().is_err());
        assert!("-p:a".parse::<DeviceName>().is_err());

        assert_eq!(
            "p:abc".parse::<DeviceName>().unwrap(),
            DeviceName {
                path: String::from("p:"),
                name: String::from("abc"),
            }
        );
        assert_eq!(
            "p:abc1".parse::<DeviceName>().unwrap(),
            DeviceName {
                path: String::from("p:"),
                name: String::from("abc1"),
            }
        );
        assert_eq!(
            "p:abc-1".parse::<DeviceName>().unwrap(),
            DeviceName {
                path: String::from("p:"),
                name: String::from("abc-1"),
            }
        );
        assert_eq!(
            "p-1:p-2:abc".parse::<DeviceName>().unwrap(),
            DeviceName {
                path: String::from("p-1:p-2:"),
                name: String::from("abc"),
            }
        );

	let dn = "p-1:p-2:abc".parse::<DeviceName>().unwrap();

        assert_eq!(dn.get_path(), "p-1:p-2");
        assert_eq!(dn.get_name(), "abc");

	assert_eq!(format!("{}", dn), "p-1:p-2:abc");
    }

    #[test]
    fn test_device_spec() {
        assert!("p:".parse::<DeviceSpec>().is_err());
        assert!("p:.".parse::<DeviceSpec>().is_err());
        assert!("p:.a".parse::<DeviceSpec>().is_err());
        assert!("p:a.".parse::<DeviceSpec>().is_err());
        assert!("p:a.123".parse::<DeviceSpec>().is_err());
        assert!("p:a.a.a".parse::<DeviceSpec>().is_err());

        assert_eq!(
            "p-1:device".parse::<DeviceSpec>().unwrap(),
            DeviceSpec {
                device: DeviceName {
                    path: String::from("p-1:"),
                    name: String::from("device"),
                },
                field: DeviceField::Value
            }
        );
        assert_eq!(
            "p:device.unit".parse::<DeviceSpec>().unwrap(),
            DeviceSpec {
                device: DeviceName {
                    path: String::from("p:"),
                    name: String::from("device"),
                },
                field: DeviceField::Unit
            }
        );
        assert_eq!(
            "path:device.value".parse::<DeviceSpec>().unwrap(),
            DeviceSpec {
                device: DeviceName {
                    path: String::from("path:"),
                    name: String::from("device"),
                },
                field: DeviceField::Value
            }
        );
        assert_eq!(
            "long:path:device.unit".parse::<DeviceSpec>().unwrap(),
            DeviceSpec {
                device: DeviceName {
                    path: String::from("long:path:"),
                    name: String::from("device"),
                },
                field: DeviceField::Unit
            }
        );
        assert_eq!(
            "long:path:device.detail".parse::<DeviceSpec>().unwrap(),
            DeviceSpec {
                device: DeviceName {
                    path: String::from("long:path:"),
                    name: String::from("device"),
                },
                field: DeviceField::Detail
            }
        );
        assert_eq!(
            "long:path:device.location".parse::<DeviceSpec>().unwrap(),
            DeviceSpec {
                device: DeviceName {
                    path: String::from("long:path:"),
                    name: String::from("device"),
                },
                field: DeviceField::Location
            }
        );
        assert_eq!(
            "long:path:device.summary".parse::<DeviceSpec>().unwrap(),
            DeviceSpec {
                device: DeviceName {
                    path: String::from("long:path:"),
                    name: String::from("device"),
                },
                field: DeviceField::Summary
            }
        );

        assert_eq!(
            "p:Device-123".parse::<DeviceSpec>().unwrap(),
            DeviceSpec {
                device: DeviceName {
                    path: String::from("p:"),
                    name: String::from("Device-123"),
                },
                field: DeviceField::Value
            }
        );
    }
}
