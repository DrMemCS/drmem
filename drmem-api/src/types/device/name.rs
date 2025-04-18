//! Defines and enforces device name requirements.
//!
//! Every device in a running instance of `drmemd` needs to have a
//! unique name. A device name is made up of segments separated with
//! colons. The last segment is the base name of the device and all
//! previous segments are the "path" of the device. A segment consists
//! of one or more UTF-8 alphanumeric or dash characters.
//!
//! In a `[[driver]]` section of `drmemd`'s configuration file, a path
//! is specified for the driver instance and the driver provides the
//! base names for its set of devices.
use crate::{types::Error, Result};
use serde_derive::Deserialize;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Deserialize, Hash, Eq)]
struct Segment(String);

impl Segment {
    // Returns `true` if the character can be used in a segment of the
    // device name.

    fn is_valid_char((idx, ch): (usize, char), len: usize) -> bool {
        ch.is_alphanumeric() || (ch == '-' && idx != 0 && idx != len - 1)
    }

    // Creates a `Segment`, if the strings contains a well-formed
    // segment name.

    fn create(s: &str) -> Result<Self> {
        if !s.is_empty() {
            if s.chars()
                .enumerate()
                .all(|v| Segment::is_valid_char(v, s.len()))
            {
                Ok(Segment(String::from(s)))
            } else {
                Err(Error::InvArgument(String::from(
                    "segment contains invalid character",
                )))
            }
        } else {
            Err(Error::InvArgument(String::from(
                "contains zero-length segment",
            )))
        }
    }
}

// This trait allows one to use `.parse::<Segment>()`.

impl FromStr for Segment {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Segment::create(s)
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.0)
    }
}

/// A type which models a device's path.
///
/// A path is a series of segments, where segments are made up of one
/// or more alphanumeric or dash characters.
#[derive(Debug, PartialEq, Clone, Deserialize, Hash, Eq)]
#[serde(try_from = "String")]
pub struct Path(Vec<Segment>);

impl Path {
    /// Creates a `Path` from a string slice. If any segment contains
    /// an invalid character, an `Err()` is returned.
    pub fn create(s: &str) -> Result<Self> {
        s.split(':')
            .map(Segment::create)
            .collect::<Result<Vec<Segment>>>()
            .map(Path)
    }
}

// This trait is defined so that the .TOML parser will use it to parse
// the device prefix field. Without this, the .TOML parser wants array
// notation for the path specification (because `Path` is a newtype
// that wraps a `Vec<>`.)

impl TryFrom<String> for Path {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        Path::create(&s)
    }
}

// This trait allows one to use `.parse::<Path>()`.

impl FromStr for Path {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Path::create(s)
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.0[0])?;
        for ii in &self.0[1..] {
            write!(f, ":{}", &ii)?
        }
        Ok(())
    }
}

/// A type representing the base name of a device.
///
/// The base name consists of one or more alphanumeric or dash
/// characters.
#[derive(Debug, PartialEq, Clone, Deserialize, Hash, Eq)]
#[serde(try_from = "String")]
pub struct Base(Segment);

impl Base {
    /// Creates a `Base` from a string slice. If it contains an
    /// invalid character, `Err()` is returned.
    pub fn create(s: &str) -> Result<Self> {
        Segment::create(s).map(Base)
    }
}

impl TryFrom<String> for Base {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        Base::create(&s)
    }
}

// This trait allows one to use `.parse::<Base>()`.

impl FromStr for Base {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Base::create(s)
    }
}

impl fmt::Display for Base {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.0)
    }
}

/// Holds a validated device name. A device name consists of a path
/// and a base name where each portion of the path is separated with a
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
/// paths. The paths provide a naming convention to organize devices.
/// The client API supports looking up device names using patterns, so
/// a logical path hierarchy can make those searches more productive.
#[derive(Debug, PartialEq, Hash, Eq, Clone, Deserialize)]
#[serde(try_from = "String")]
pub struct Name {
    path: Path,
    base: Base,
}

impl Name {
    /// Creates an instance of `Name`, if the provided string
    /// describes a well-formed device name.
    pub fn create(s: &str) -> Result<Name> {
        match s
            .split(':')
            .map(Segment::create)
            .collect::<Result<Vec<Segment>>>()
        {
            Ok(segments) if segments.len() < 2 => Err(Error::InvArgument(
                String::from("device name requires a path and base name"),
            )),
            Ok(segments) => Ok(Name {
                path: Path(segments[0..segments.len() - 1].to_vec()),
                base: Base(segments[segments.len() - 1].clone()),
            }),
            Err(e) => Err(e),
        }
    }

    /// Builds a device name from `Path` and `Base` components.
    pub fn build(path: Path, base: Base) -> Name {
        Name { path, base }
    }

    /// Returns the path of the device name without the trailing ':'.
    pub fn get_path(&self) -> Path {
        self.path.clone()
    }

    /// Returns the base name of the device.
    pub fn get_name(&self) -> Base {
        self.base.clone()
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", &self.path, &self.base)
    }
}

impl TryFrom<String> for Name {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        Name::create(&s)
    }
}

// This trait allows one to use `.parse::<Name>()`.

impl FromStr for Name {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Name::create(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_segment() {
        assert!("".parse::<Segment>().is_err());
        assert!(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
                .parse::<Segment>()
                .is_ok()
        );
        assert!("a-b".parse::<Segment>().is_ok());
        assert!("a:b".parse::<Segment>().is_err());
        assert!("-a".parse::<Segment>().is_err());
        assert!("a-".parse::<Segment>().is_err());
        assert!(" ".parse::<Segment>().is_err());
        assert_eq!(format!("{}", "a-b".parse::<Segment>().unwrap()), "a-b");

        // Check non-ASCII entries.

        assert!("٣".parse::<Segment>().is_ok());
        assert!("温度".parse::<Segment>().is_ok());
        assert!("🤖".parse::<Segment>().is_err());
    }

    #[test]
    fn test_base() {
        assert_eq!(format!("{}", "a-b".parse::<Base>().unwrap()), "a-b");
        assert!("a:b".parse::<Base>().is_err());
    }

    #[test]
    fn test_path() {
        assert!("".parse::<Path>().is_err());
        assert!("basement:🤖".parse::<Path>().is_err());

        assert_eq!(format!("{}", "a-b".parse::<Path>().unwrap()), "a-b");
        assert_eq!(format!("{}", "a:b".parse::<Path>().unwrap()), "a:b");
        assert_eq!(format!("{}", "a:b:c".parse::<Path>().unwrap()), "a:b:c");
        assert_eq!(
            format!("{}", "家:温度".parse::<Path>().unwrap()),
            "家:温度"
        );
    }

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
                path: Path::create("p").unwrap(),
                base: Base::create("abc").unwrap(),
            }
        );
        assert_eq!(
            "p:abc1".parse::<Name>().unwrap(),
            Name {
                path: Path::create("p").unwrap(),
                base: Base::create("abc1").unwrap(),
            }
        );
        assert_eq!(
            "p:abc-1".parse::<Name>().unwrap(),
            Name {
                path: Path::create("p").unwrap(),
                base: Base::create("abc-1").unwrap(),
            }
        );
        assert_eq!(
            "p-1:p-2:abc".parse::<Name>().unwrap(),
            Name {
                path: Path::create("p-1:p-2").unwrap(),
                base: Base::create("abc").unwrap(),
            }
        );

        let dn = "p-1:p-2:abc".parse::<Name>().unwrap();

        assert_eq!(dn.get_path(), Path::create("p-1:p-2").unwrap());
        assert_eq!(dn.get_name(), Base::create("abc").unwrap());

        assert_eq!(format!("{}", dn), "p-1:p-2:abc");
    }
}
