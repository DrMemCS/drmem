use drmem_api::{
    driver::{self, Reporter},
    Result,
};
use std::convert::Infallible;

use super::{config, device};

// The state of a driver instance.

pub enum Instance {
    Primed(f64),
    Reset(f64),
}

impl Instance {
    pub const NAME: &'static str = "integrate";

    pub const SUMMARY: &'static str = "Accumulates values that have occurred";

    pub const DESCRIPTION: &'static str = include_str!("drv_integrate.md");

    /// Creates a new, idle `Instance`.
    pub fn new() -> Instance {
        Instance::Primed(0.0)
    }

    pub fn accumulate(&mut self, val: f64) {
        match self {
            Instance::Primed(v) | Instance::Reset(v) => *v = *v + val,
        }
    }

    pub fn reset(&mut self, val: bool) -> bool {
        match self {
            Instance::Primed(_) => {
                if val {
                    *self = Instance::Reset(0.0);
                    true
                } else {
                    false
                }
            }
            Instance::Reset(v) => {
                if !val {
                    *self = Instance::Primed(*v);
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn get_sum(&self) -> f64 {
        match self {
            Instance::Primed(v) => *v,
            Instance::Reset(v) => *v,
        }
    }
}

impl<R: Reporter> driver::API<R> for Instance {
    type Config = config::Params;
    type HardwareType = device::Set<R>;

    async fn create_instance(_cfg: &Self::Config) -> Result<Box<Self>> {
        Ok(Box::new(Instance::new()))
    }

    async fn run(&mut self, devices: &mut Self::HardwareType) -> Infallible {
        devices.d_sum.report_update(self.get_sum()).await;

        loop {
            let mut report = true;

            #[rustfmt::skip]
            tokio::select! {
                Some((b, reply)) = devices.d_data.next_setting() => {
                    reply.ok(b);
                    devices.d_data.report_update(b).await;
                    self.accumulate(b);
                }
                Some((b, reply)) = devices.d_reset.next_setting() => {
                    reply.ok(b);
                    devices.d_reset.report_update(b).await;
                    report = self.reset(b);
                }
            }

            if report {
                devices.d_sum.report_update(self.get_sum()).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_changes() {
        let mut accum = Instance::new();

        assert_eq!(accum.get_sum(), 0.0);
        accum.accumulate(1.0);
        assert_eq!(accum.get_sum(), 1.0);
        accum.accumulate(2.0);
        assert_eq!(accum.get_sum(), 3.0);
        accum.reset(false);
        assert_eq!(accum.get_sum(), 3.0);
        accum.reset(true);
        assert_eq!(accum.get_sum(), 0.0);
        accum.accumulate(1.0);
        assert_eq!(accum.get_sum(), 1.0);
        accum.accumulate(2.0);
        assert_eq!(accum.get_sum(), 3.0);
        accum.reset(true);
        assert_eq!(accum.get_sum(), 3.0);
    }
}
