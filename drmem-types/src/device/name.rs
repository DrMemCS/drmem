use std::fmt;
use std::str::FromStr;
use regex::Regex;
use lazy_static::lazy_static;
use crate::Error;

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
