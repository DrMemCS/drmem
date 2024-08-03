use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::{
    convert::Infallible, future::Future, ops::RangeInclusive, pin::Pin,
    sync::Arc,
};
use tokio::sync::Mutex;
use tokio_stream::StreamExt;

#[derive(Debug, PartialEq)]
struct Entry(RangeInclusive<i32>, device::Value);

pub struct Instance {
    init_index: Option<i32>,
    def_val: device::Value,
    values: Vec<Entry>,
}

pub struct Devices {
    d_output: driver::ReportReading<device::Value>,
    d_index: driver::ReportReading<i32>,
    s_index: driver::SettingStream<i32>,
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
            Some(_) => Err(Error::ConfigError(
                "'initial' config parameter should be a 32-bit integer".into(),
            )),
            None => Ok(None),
        }
    }

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
                        "`default` config paramter : {}",
                        e
                    ))
                })
            })
    }

    fn to_entry(tbl: &toml::Table, def: &device::Value) -> Result<Entry> {
        if let Some(toml::value::Value::Integer(start)) = tbl.get("start") {
            if let Some(value) = tbl.get("value") {
                let value = device::Value::try_from(value).map_err(|_| {
                    Error::ConfigError(
                        "`values` array entry contains unknown `value` type"
                            .into(),
                    )
                })?;

                if !def.is_same_type(&value) {
                    return Err(Error::ConfigError(
			"all values in `values` array entries must be the same type as the default value"
			    .into()
		    ));
                }

                let start = *start as i32;

                match tbl.get("end") {
                    Some(toml::value::Value::Integer(end)) => {
                        let end = *end as i32;

                        Ok(Entry(start.min(end)..=start.max(end), value))
                    }
                    Some(_) => Err(Error::ConfigError(
                        "`values` array entry has a bad `end` value".into(),
                    )),
                    None => Ok(Entry(start..=start, value)),
                }
            } else {
                Err(Error::ConfigError(
                    "`values` array entry missing `value`".into(),
                ))
            }
        } else {
            Err(Error::ConfigError(
                "`values` array entry missing `start`".into(),
            ))
        }
    }

    fn get_cfg_values(
        cfg: &DriverConfig,
        def: &device::Value,
    ) -> Result<Vec<Entry>> {
        match cfg.get("values") {
            Some(toml::value::Value::Array(arr)) if !arr.is_empty() => {
                let mut result = vec![];

                for entry in arr {
                    match entry {
                        toml::value::Value::Table(tbl) => {
                            result.push(Self::to_entry(tbl, def)?)
                        }
                        _ => {
                            return Err(Error::ConfigError(
                                "`values` array contains a non-table".into(),
                            ))
                        }
                    }
                }

                Ok(result)
            }
            Some(_) => Err(Error::ConfigError(
                "`values` config parameter should be a non-empty array of maps"
                    .into(),
            )),
            None => Err(Error::ConfigError(
                "`values` config parameter is missing".into(),
            )),
        }
    }

    // Find the entry that contains the index. Return the associated
    // value. If no entry matches, return the error value.

    fn map_to(&self, idx: i32) -> device::Value {
        self.values
            .iter()
            .find(|e| e.0.contains(&idx))
            .map(|e| e.1.clone())
            .unwrap_or_else(|| self.def_val.clone())
    }
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let output_name = "output".parse::<device::Base>().unwrap();
        let index_name = "index".parse::<device::Base>().unwrap();

        Box::pin(async move {
            // Define the devices managed by this driver.
            //
            // This first device is the output of the map.

            let (d_output, _) =
                core.add_ro_device(output_name, None, max_history).await?;

            // This device is settable. Any setting is forwarded to
            // the backend.

            let (d_index, s_index, _) =
                core.add_rw_device(index_name, None, max_history).await?;

            Ok(Devices {
                d_output,
                d_index,
                s_index,
            })
        })
    }

    fn create_instance(
        cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        let init_index = Instance::get_cfg_init_val(cfg);
        let def_value = Instance::get_cfg_def_val(cfg);
        let values = if let Ok(ref def_value) = def_value {
            Instance::get_cfg_values(cfg, def_value)
        } else {
            Ok(vec![])
        };

        Box::pin(async move {
            Ok(Box::new(Instance::new(init_index?, def_value?, values?)))
        })
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            let mut devices = devices.lock().await;

            // If we have an initial value, use it.

            if let Some(idx) = self.init_index {
                // Send the updated values to the backend.

                (devices.d_output)(self.map_to(idx)).await;
                (devices.d_index)(idx).await
            } else {
                (devices.d_output)(self.def_val.clone()).await;
            }

            // The driver blocks, waiting for a new index. As long as
            // our setting channel is healthy, we handle each setting.

            while let Some((v, reply)) = devices.s_index.next().await {
                // Send the reply to the setter.

                reply(Ok(v));

                // Send the updated values to the backend.

                (devices.d_output)(self.map_to(v)).await;
                (devices.d_index)(v).await
            }
            panic!("can no longer receive settings");
        };

        Box::pin(fut)
    }
}

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
        }
    }
}
