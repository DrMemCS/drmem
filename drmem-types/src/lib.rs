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
use std::convert::From;
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
pub enum Error {
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

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "item not found"),
            Error::InUse => write!(f, "item is in use"),
            Error::DeviceDefined(name) => {
                write!(f, "device {} is already defined", &name)
            }
            Error::MissingPeer(detail) => {
                write!(f, "{} is missing peer", detail)
            }
            Error::TypeError => write!(f, "incorrect type"),
            Error::InvArgument(s) => write!(f, "{}", s),
            Error::DbCommunicationError => {
                write!(f, "db communication error")
            }
            Error::AuthenticationError => write!(f, "permission error"),
            Error::OperationError => {
                write!(f, "couldn't complete operation")
            }
            Error::BadConfig => write!(f, "bad configuration"),
            Error::UnknownError => write!(f, "unhandled error"),
        }
    }
}

pub mod device;

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

    pub fn create(s: &str) -> Result<DeviceName, Error> {
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
            Err(Error::InvArgument("invalid device path/name"))
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
    type Err = Error;

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
    fn xlat_field(s: &str) -> Result<DeviceField, Error> {
        match s {
            "value" => Ok(DeviceField::Value),
            "unit" => Ok(DeviceField::Unit),
            "detail" => Ok(DeviceField::Detail),
            "summary" => Ok(DeviceField::Summary),
            "location" => Ok(DeviceField::Location),
            _ => Err(Error::InvArgument("invalid field name")),
        }
    }

    /// Creates an instance of `DeviceSpec` if the provided string
    /// describes a well-formed device specification.

    pub fn create(s: &str) -> Result<DeviceSpec, Error> {
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
        Err(Error::InvArgument("invalid device specification"))
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
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DeviceSpec::create(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
