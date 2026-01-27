use drmem_api::{device, driver::DriverConfig, Error};

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct Entry {
    pub name: device::Base,
    pub initial: device::Value,
}

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct Params {
    pub vars: Vec<Entry>,
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

#[cfg(test)]
mod test {
    use super::super::config;
    use drmem_api::{driver::DriverConfig, Error, Result};

    fn mk_cfg(text: &str) -> Result<config::Params> {
        Into::<DriverConfig>::into(
            toml::from_str::<toml::value::Table>(text)
                .map_err(|e| Error::ConfigError(format!("{}", e)))?,
        )
        .parse_into()
    }

    #[test]
    fn test_configuration() {
        use super::device;
        use toml::{Table, Value};

        // Test for an empty Map or a Map that doesn't have the "vars"
        // key or a map with "vars" whose value isn't a map or is a
        // map but is empty or has a value, but it's not an array. All
        // of these are errors.

        {
            assert!(mk_cfg("vars = [{initial = true}]").is_err());
            assert!(mk_cfg("vars = [{name = \"var\"}]").is_err());
            assert!(mk_cfg("vars = [{junk = \"var\"}]").is_err());
            assert!(mk_cfg("vars = [{name = \"var\", initial = true}]").is_ok());

            assert_eq!(
                mk_cfg(
                    "
vars = [{name = \"v1\", initial = true},
        {name = \"v2\", initial = 100}]"
                ),
                Ok(config::Params {
                    vars: vec![
                        config::Entry {
                            name: "v1".try_into().unwrap(),
                            initial: device::Value::Bool(true)
                        },
                        config::Entry {
                            name: "v2".try_into().unwrap(),
                            initial: device::Value::Int(100)
                        }
                    ]
                })
            );
        }

        // Now make sure the config code creates a single memory
        // device correctly. We'll deal with sets later.

        {
            let mut test_set: Vec<(&'static str, Value, device::Value)> = vec![
                ("flag", Value::Boolean(true), device::Value::Bool(true)),
                ("int-val", Value::Integer(100), device::Value::Int(100)),
                ("flt-val", Value::Float(50.0), device::Value::Flt(50.0)),
                (
                    "str-val",
                    Value::String("Hello".into()),
                    device::Value::Str("Hello".into()),
                ),
                (
                    "clr-val",
                    Value::String("#ffffff".into()),
                    device::Value::Color(palette::LinSrgba::new(
                        255, 255, 255, 255,
                    )),
                ),
            ];

            for entry in test_set.drain(..) {
                let mut tbl = Table::new();
                let _ = tbl.insert("name".into(), entry.0.try_into().unwrap());
                let _ = tbl.insert("initial".into(), entry.1.clone());
                let mut map = Table::new();

                map.insert(
                    "vars".into(),
                    Value::Array(vec![Value::Table(tbl)]),
                );

                let result: config::Params =
                    Into::<DriverConfig>::into(map).parse_into().unwrap();

                assert!(result.vars.len() == 1);
                assert_eq!(result.vars[0].name.to_string(), entry.0);
                assert_eq!(result.vars[0].initial, entry.2);
            }
        }
    }
}
