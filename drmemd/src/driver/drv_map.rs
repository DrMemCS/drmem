use drmem_api::{
    device,
    driver::{self, DriverConfig, ResettableState},
    Result,
};
use std::{convert::Infallible, ops::RangeInclusive};
use tokio::time::Duration;

mod config {
    use drmem_api::{device, driver::DriverConfig, Error, Result};

    #[derive(PartialEq, serde::Deserialize, Debug)]
    pub struct Entry {
        pub start: i32,
        pub end: Option<i32>,
        pub value: device::Value,
    }

    #[derive(serde::Deserialize)]
    pub struct InstanceConfig {
        pub initial: Option<i32>,
        pub default: device::Value,
        pub values: Vec<Entry>,
    }

    // Convert the TOML Table into `InstanceConfig`.

    pub fn parse(cfg: &DriverConfig) -> Result<InstanceConfig> {
        let mut cfg: InstanceConfig = cfg.parse_into()?;

        // Sort the entries by the value of the start index.

        cfg.values.sort_by(|a, b| a.start.cmp(&b.start));

        // Now check to see if any ranges overlap. If so, that's an
        // error.

        if cfg
            .values
            .windows(2)
            .any(|e| e[0].end.unwrap_or(e[0].start) >= e[1].start)
        {
            return Err(Error::ConfigError(
                "`values` array contains overlapping ranges".into(),
            ));
        }
        Ok(cfg)
    }
}

#[derive(Debug, PartialEq)]
struct Entry(RangeInclusive<i32>, device::Value);

pub struct Instance {
    init_index: Option<i32>,
    def_val: device::Value,
    values: Vec<Entry>,
}

impl Instance {
    pub const NAME: &'static str = "map";

    pub const SUMMARY: &'static str = "Maps a range of indices to a value.";

    pub const DESCRIPTION: &'static str = include_str!("drv_map.md");

    /// Creates a new `Instance` instance.
    fn new(cfg: &DriverConfig) -> Result<Instance> {
        let cfg = config::parse(cfg)?;

        Ok(Instance {
            init_index: cfg.initial,
            def_val: cfg.default,
            values: cfg.values.iter().map(Instance::to_entry).collect(),
        })
    }

    // Helper method to convert a TOML Table into an `Entry` type.

    fn to_entry(e: &config::Entry) -> Entry {
        match e {
            config::Entry {
                start,
                end: None,
                value,
            } => Entry(*start..=*start, value.clone()),
            config::Entry {
                start,
                end: Some(end),
                value,
            } => Entry(*start.min(end)..=*start.max(end), value.clone()),
        }
    }

    // Find the entry that contains the index. Return the associated
    // value. If no entry matches, return the error value.

    fn map_to(&self, idx: i32) -> device::Value {
        use std::cmp::Ordering;

        self.values
            .binary_search_by(|e| {
                if idx < *e.0.start() {
                    Ordering::Less
                } else if idx > *e.0.end() {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            })
            .map(|v| self.values[v].1.clone())
            .unwrap_or_else(|_| self.def_val.clone())
    }
}

pub struct Devices {
    d_output: driver::ReadOnlyDevice<device::Value>,
    d_index: driver::ReadWriteDevice<i32>,
}

impl driver::Registrator for Devices {
    async fn register_devices(
        core: &mut driver::RequestChan,
        _cfg: &DriverConfig,
        _override_timeout: Option<Duration>,
        max_history: Option<usize>,
    ) -> Result<Self> {
        // Define the devices managed by this driver.
        //
        // This first device is the output of the map.

        let d_output = core.add_ro_device("output", None, max_history).await?;

        // This device is settable. Any setting is forwarded to
        // the backend.

        let d_index = core.add_rw_device("index", None, max_history).await?;

        Ok(Devices { d_output, d_index })
    }
}

impl driver::API for Instance {
    type HardwareType = Devices;

    async fn create_instance(cfg: &DriverConfig) -> Result<Box<Self>> {
        Instance::new(cfg).map(Box::new)
    }

    async fn run(&mut self, devices: &mut Self::HardwareType) -> Infallible {
        // If we have an initial value, use it.

        if let Some(idx) = self.init_index {
            // Send the updated values to the backend.

            devices.d_output.report_update(self.map_to(idx)).await;
            devices.d_index.report_update(idx).await
        } else {
            devices.d_output.report_update(self.def_val.clone()).await;
        }

        // The driver blocks, waiting for a new index. As long as our
        // setting channel is healthy, we handle each setting.

        while let Some((v, reply)) = devices.d_index.next_setting().await {
            // Send the reply to the setter.

            reply.ok(v);

            // Send the updated values to the backend.

            devices.d_output.report_update(self.map_to(v)).await;
            devices.d_index.report_update(v).await
        }
        panic!("can no longer receive settings");
    }
}

impl ResettableState for Devices {}

#[cfg(test)]
mod tests {
    use super::config;
    use drmem_api::{device, driver::DriverConfig, Error, Result};

    // Tries to build an `InstanceConfig` from a `&str`.

    fn make_cfg(text: &str) -> Result<config::InstanceConfig> {
        config::parse(&Into::<DriverConfig>::into(
            toml::from_str::<toml::value::Table>(text)
                .map_err(|e| Error::ConfigError(format!("{}", e)))?,
        ))
    }

    #[test]
    fn test_cfg_initial() {
        {
            let cfg = make_cfg(
                "default = false
values = []
",
            )
            .unwrap();

            assert_eq!(cfg.initial, None);
        }
        {
            let cfg = make_cfg(
                "initial = \"hello\"
default = false
values = []
",
            );

            assert!(cfg.is_err());
        }
        {
            let cfg = make_cfg(
                "initial = 100
default = false
values = []
",
            )
            .unwrap();

            assert_eq!(cfg.initial, Some(100));
        }
    }

    #[test]
    fn test_cfg_default() {
        let cfg = make_cfg(
            "initial = 100
default = \"hello\"
values = []
",
        )
        .unwrap();

        assert_eq!(cfg.default, device::Value::Str("hello".into()));
    }

    #[test]
    fn test_bad_cfg_values() {
        // Missing the 'values' array.

        {
            let cfg = make_cfg("default = 100");

            assert!(cfg.is_err());
        }

        // Missing the 'start' field.

        {
            let cfg = make_cfg(
                "default = 100
values = [{ value = false }]",
            );

            assert!(cfg.is_err());
        }

        // Missing a 'value' field.

        {
            let cfg = make_cfg(
                "default = 100
values = [{ start = 0 }]",
            );

            assert!(cfg.is_err());
        }

        // The first and third entry's ranges overlap.

        {
            let cfg = make_cfg(
                "default = 100
values = [{ start = 2, end = 7, value = \"there\" },
          { start = 8, end = 11, value = \"world\" },
          { start = 0, end = 3, value = \"hello\" }]",
            );

            assert!(cfg.is_err());
        }

        // The second and third entry's ranges share an endpoint
        // (i.e. overlap.).

        {
            let cfg = make_cfg(
                "default = 100
values = [{ start = 8, end = 11, value = \"there\" },
          { start = 3, end = 7, value = \"world\" },
          { start = 0, end = 3, value = \"hello\" }]",
            );

            assert!(cfg.is_err());
        }
    }

    #[test]
    fn test_good_cfg_values() {
        {
            let cfg = make_cfg(
                "default = 100
values = [{ start = 0, value = false }]",
            )
            .unwrap();

            assert_eq!(
                cfg.values,
                vec![config::Entry {
                    start: 0,
                    end: None,
                    value: device::Value::Bool(false)
                }]
            );
        }

        {
            let cfg = make_cfg(
                "default = 100
values = [{ start = 0, end = 1, value = \"hello\" }]",
            )
            .unwrap();

            assert_eq!(
                cfg.values,
                vec![config::Entry {
                    start: 0,
                    end: Some(1),
                    value: device::Value::Str("hello".into())
                }],
            );
        }

        {
            let cfg = make_cfg(
                "default = 100
values = [{ start = 4, end = 7, value = \"there\" },
          { start = 8, end = 11, value = \"world\" },
          { start = 0, end = 3, value = \"hello\" }]",
            )
            .unwrap();

            assert_eq!(
                cfg.values,
                vec![
                    config::Entry {
                        start: 0,
                        end: Some(3),
                        value: device::Value::Str("hello".into())
                    },
                    config::Entry {
                        start: 4,
                        end: Some(7),
                        value: device::Value::Str("there".into())
                    },
                    config::Entry {
                        start: 8,
                        end: Some(11),
                        value: device::Value::Str("world".into())
                    }
                ]
            );
        }
    }
}
