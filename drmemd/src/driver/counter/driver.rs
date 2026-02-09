use drmem_api::{
    driver::{self, Reporter},
    Result,
};
use std::convert::Infallible;

use super::{config, device};

// The state of a driver instance.

pub enum Instance {
    Waiting(i32),
    Tripped(i32),
}

impl Instance {
    pub const NAME: &'static str = "counter";

    pub const SUMMARY: &'static str = "Counts events that have occurred";

    pub const DESCRIPTION: &'static str = include_str!("drv_counter.md");

    /// Creates a new, idle `Instance`.
    pub fn new() -> Instance {
        Instance::Waiting(0)
    }

    pub fn incrememt(&mut self, val: bool) -> Option<i32> {
        match self {
            Instance::Waiting(count) => {
                if val {
                    let temp = *count + 1;

                    *self = Instance::Tripped(temp);
                    return Some(temp);
                }
            }
            Instance::Tripped(count) => {
                if !val {
                    *self = Instance::Waiting(*count);
                }
            }
        }
        None
    }

    pub fn reset(&mut self, val: bool) {
        if val {
            *self = Instance::Waiting(0);
        }
    }

    pub fn get_count(&self) -> i32 {
        match self {
            Instance::Waiting(count) => *count,
            Instance::Tripped(count) => *count,
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
        devices.d_count.report_update(self.get_count()).await;

        loop {
            #[rustfmt::skip]
            tokio::select! {
                Some((b, reply)) = devices.d_increment.next_setting() => {
                    // Record all settings.

                    devices.d_increment.report_update(b).await;

                    // Possibly update the count. If it is incrmented, report it.

                    if let Some(count) = self.incrememt(b) {
                        devices.d_count.report_update(count).await;
                    }

                    // Notify the client.

                    reply.ok(b)
                }
                Some((b, reply)) = devices.d_reset.next_setting() => {
                    // Record all settings.

                    devices.d_reset.report_update(b).await;

                    // Reset and report the new count.

                    self.reset(b);
                    devices.d_count.report_update(0).await;

                    // Notify the client.

                    reply.ok(b);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_changes() {
        let mut counter = Instance::new();

        assert_eq!(counter.get_count(), 0);
        assert_eq!(counter.incrememt(false), None);
        assert_eq!(counter.incrememt(true), Some(1));
        assert_eq!(counter.incrememt(false), None);
        assert_eq!(counter.incrememt(true), Some(2));
        assert_eq!(counter.incrememt(false), None);
        counter.reset(false);
        assert_eq!(counter.get_count(), 2);
        counter.reset(true);
        assert_eq!(counter.get_count(), 0);
        counter.reset(false);
        assert_eq!(counter.get_count(), 0);
        assert_eq!(counter.incrememt(false), None);
        assert_eq!(counter.incrememt(true), Some(1));
    }
}
