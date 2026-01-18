use drmem_api::{
    device,
    driver::{self, DriverConfig, ResettableState},
    Error, Result,
};
use std::{convert::Infallible, ops::RangeInclusive};
use tokio::time::Duration;

#[derive(Debug, PartialEq)]
struct Entry(RangeInclusive<i32>, device::Value);

fn config_err<T>(msg: &str) -> Result<T> {
    Err(Error::ConfigError(msg.into()))
}

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
    fn new(
        init_index: Option<i32>,
        def_val: device::Value,
        values: Vec<Entry>,
    ) -> Instance {
        Instance {
            init_index,
            def_val,
            values,
        }
    }

    // Gets the initial value of the index from the configuration.

    fn get_cfg_init_val(cfg: &DriverConfig) -> Result<Option<i32>> {
        match cfg.get("initial") {
            Some(toml::value::Value::Integer(v))
                if *v >= (i32::MIN as i64) && *v <= (i32::MAX as i64) =>
            {
                Ok(Some(*v as i32))
            }
            Some(_) => config_err(
                "'initial' config parameter should be a 32-bit integer",
            ),
            None => Ok(None),
        }
    }

    // Retrieve the default value from the configuration.

    fn get_cfg_def_val(cfg: &DriverConfig) -> Result<device::Value> {
        cfg.get("default")
            .ok_or_else(|| {
                Error::ConfigError(
                    "`default` config parameter is missing".into(),
                )
            })
            .and_then(|v| {
                device::Value::try_from(v).map_err(|e| {
                    Error::ConfigError(format!(
                        "`default` config paramter : {e}"
                    ))
                })
            })
    }

    // Helper method to convert a TOML Table into an `Entry` type.

    fn to_entry(tbl: &toml::Table, def: &device::Value) -> Result<Entry> {
        // The table *must* have a key called "start" and the value
        // *must* be an integer.

        if let Some(toml::value::Value::Integer(start)) = tbl.get("start") {
            // The table *must* have a key called "value".

            if let Some(value) = tbl.get("value") {
                // The value associated with the "value" key *must* be
                // convertable into a `device::Value` type.

                let value = device::Value::try_from(value).map_err(|_| {
                    Error::ConfigError(
                        "`values` array entry contains unknown `value` type"
                            .into(),
                    )
                })?;

                // All values in the array must be of the same type as
                // the default value (it's hard to imagine a device
                // correctly handling different types.)

                if def.is_same_type(&value) {
                    // Convert to `i32`.

                    let start = *start as i32;

                    // The "end" key is optional. If missing, we use
                    // "start" as the end. If it is present, however,
                    // it must be an integer.

                    match tbl.get("end") {
                        Some(toml::value::Value::Integer(end)) => {
                            let end = *end as i32;

                            // Make sure the limits of the range are in
                            // ascending order.

                            Ok(Entry(start.min(end)..=start.max(end), value))
                        }
                        Some(_) => config_err(
                            "`values` array entry has a bad `end` value",
                        ),
                        None => Ok(Entry(start..=start, value)),
                    }
                } else {
                    config_err(
			"all values in `values` array entries must be the same type as the default value"
		    )
                }
            } else {
                config_err("`values` array entry missing `value`")
            }
        } else {
            config_err("`values` array entry missing `start`")
        }
    }

    // Retrieve the array of entries that make up the mapping table.

    fn get_cfg_values(
        cfg: &DriverConfig,
        def: &device::Value,
    ) -> Result<Vec<Entry>> {
        match cfg.get("values") {
            Some(toml::value::Value::Array(arr)) if !arr.is_empty() => {
                let mut result: Vec<Entry> = arr
                    .iter()
                    .map(|entry| {
                        if let toml::value::Value::Table(tbl) = entry {
                            Self::to_entry(tbl, def)
                        } else {
                            config_err("`values` array contains a non-table")
                        }
                    })
                    .collect::<Result<Vec<Entry>>>()?;

                // Sort the vector by the lower bounds of the range.

                result.sort_by(|a, b| a.0.start().cmp(b.0.start()));

                // If any adjacent ranges overlap, return an error.

                if result.windows(2).any(|e| e[0].0.end() >= e[1].0.start()) {
                    config_err("`values` array contains overlapping ranges")
                } else {
                    Ok(result)
                }
            }
            Some(_) => config_err(
                "`values` config parameter should be a non-empty array of maps",
            ),
            None => config_err("`values` config parameter is missing"),
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
    async fn register_devices<'a>(
        core: &'a mut driver::RequestChan,
        _cfg: &DriverConfig,
        _override_timeout: Option<Duration>,
        max_history: Option<usize>,
    ) -> Result<Self> {
        let output_name = "output".parse::<device::Base>().unwrap();
        let index_name = "index".parse::<device::Base>().unwrap();

        // Define the devices managed by this driver.
        //
        // This first device is the output of the map.

        let d_output =
            core.add_ro_device(output_name, None, max_history).await?;

        // This device is settable. Any setting is forwarded to
        // the backend.

        let d_index = core.add_rw_device(index_name, None, max_history).await?;

        Ok(Devices { d_output, d_index })
    }
}

impl driver::API for Instance {
    type HardwareType = Devices;

    async fn create_instance(cfg: &DriverConfig) -> Result<Box<Self>> {
        let init_index = Instance::get_cfg_init_val(cfg);
        let def_value = Instance::get_cfg_def_val(cfg);
        let values = if let Ok(ref def_value) = def_value {
            Instance::get_cfg_values(cfg, def_value)
        } else {
            Ok(vec![])
        };

        Ok(Box::new(Instance::new(init_index?, def_value?, values?)))
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
    use super::{Entry, Instance};
    use drmem_api::device;
    use toml::value::{Table, Value};

    #[test]
    fn test_cfg_initial() {
        let mut tbl = Table::new();

        assert_eq!(Instance::get_cfg_init_val(&tbl), Ok(None));

        let _ = tbl.insert("initial".into(), Value::String("hello".into()));

        assert!(Instance::get_cfg_init_val(&tbl).is_err());

        let _ = tbl.insert("initial".into(), Value::Integer(100));

        assert_eq!(Instance::get_cfg_init_val(&tbl), Ok(Some(100)));
    }

    #[test]
    fn test_cfg_default() {
        let mut tbl = Table::new();

        assert!(Instance::get_cfg_def_val(&tbl).is_err());

        let _ = tbl.insert("default".into(), Value::String("hello".into()));

        assert_eq!(
            Instance::get_cfg_def_val(&tbl),
            Ok(device::Value::Str("hello".into()))
        );
    }

    fn build_table(
        start: Option<i64>,
        end: Option<i64>,
        val: Option<Value>,
    ) -> Value {
        let mut tbl = Table::new();

        if let Some(start) = start {
            let _ = tbl.insert("start".into(), Value::Integer(start));
        }

        if let Some(val) = val {
            let _ = tbl.insert("value".into(), val);
        }

        if let Some(end) = end {
            let _ = tbl.insert("end".into(), Value::Integer(end));
        }

        Value::Table(tbl)
    }

    #[test]
    fn test_cfg_values() {
        {
            let def_val = device::Value::Str("world".into());
            let mut tbl = Table::new();

            // First test for bad configurations.
            //
            // This tests that a missing "values" key is an error.

            assert!(Instance::get_cfg_values(&tbl, &def_val).is_err());

            // Tests if the range entry is missing a "start" key.

            let _ = tbl.insert(
                "values".into(),
                Value::Array(vec![build_table(None, None, None)]),
            );

            assert!(Instance::get_cfg_values(&tbl, &def_val).is_err());

            // Tests if the range entry is missing a "value" key.

            let _ = tbl.insert(
                "values".into(),
                Value::Array(vec![build_table(Some(0), None, None)]),
            );

            assert!(Instance::get_cfg_values(&tbl, &def_val).is_err());
        }

        // Now test good configurations.

        {
            let def_val = device::Value::Str("world".into());
            let mut tbl = Table::new();

            // Test that providing all fields generates a entry.

            let _ = tbl.insert(
                "values".into(),
                Value::Array(vec![build_table(
                    Some(0),
                    Some(10),
                    Some(Value::String("hello".into())),
                )]),
            );

            assert_eq!(
                Instance::get_cfg_values(&tbl, &def_val).unwrap(),
                vec![Entry(0..=10, device::Value::Str("hello".into()))]
            );

            // Test that omitting the "end" set it equal to "start".

            let _ = tbl.insert(
                "values".into(),
                Value::Array(vec![build_table(
                    Some(0),
                    None,
                    Some(Value::String("hello".into())),
                )]),
            );

            assert_eq!(
                Instance::get_cfg_values(&tbl, &def_val).unwrap(),
                vec![Entry(0..=0, device::Value::Str("hello".into()))]
            );

            // Test that the function sorts the results.

            let _ = tbl.insert(
                "values".into(),
                Value::Array(vec![
                    build_table(
                        Some(0),
                        Some(3),
                        Some(Value::String("hello".into())),
                    ),
                    build_table(
                        Some(4),
                        Some(7),
                        Some(Value::String("there".into())),
                    ),
                    build_table(
                        Some(8),
                        Some(11),
                        Some(Value::String("world".into())),
                    ),
                ]),
            );

            assert_eq!(
                Instance::get_cfg_values(&tbl, &def_val).unwrap(),
                vec![
                    Entry(0..=3, device::Value::Str("hello".into())),
                    Entry(4..=7, device::Value::Str("there".into())),
                    Entry(8..=11, device::Value::Str("world".into()))
                ]
            );

            let _ = tbl.insert(
                "values".into(),
                Value::Array(vec![
                    build_table(
                        Some(8),
                        Some(11),
                        Some(Value::String("world".into())),
                    ),
                    build_table(
                        Some(4),
                        Some(7),
                        Some(Value::String("there".into())),
                    ),
                    build_table(
                        Some(0),
                        Some(3),
                        Some(Value::String("hello".into())),
                    ),
                ]),
            );

            assert_eq!(
                Instance::get_cfg_values(&tbl, &def_val).unwrap(),
                vec![
                    Entry(0..=3, device::Value::Str("hello".into())),
                    Entry(4..=7, device::Value::Str("there".into())),
                    Entry(8..=11, device::Value::Str("world".into()))
                ]
            );

            // Test that it rejects an array with overlapping ranges.

            let _ = tbl.insert(
                "values".into(),
                Value::Array(vec![
                    build_table(
                        Some(8),
                        Some(11),
                        Some(Value::String("world".into())),
                    ),
                    build_table(
                        Some(2),
                        Some(7),
                        Some(Value::String("there".into())),
                    ),
                    build_table(
                        Some(0),
                        Some(3),
                        Some(Value::String("hello".into())),
                    ),
                ]),
            );

            assert!(Instance::get_cfg_values(&tbl, &def_val).is_err());

            let _ = tbl.insert(
                "values".into(),
                Value::Array(vec![
                    build_table(
                        Some(8),
                        Some(11),
                        Some(Value::String("world".into())),
                    ),
                    build_table(
                        Some(3),
                        Some(7),
                        Some(Value::String("there".into())),
                    ),
                    build_table(
                        Some(0),
                        Some(3),
                        Some(Value::String("hello".into())),
                    ),
                ]),
            );

            assert!(Instance::get_cfg_values(&tbl, &def_val).is_err());
        }
    }
}
