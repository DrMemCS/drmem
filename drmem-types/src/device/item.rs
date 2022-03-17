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
