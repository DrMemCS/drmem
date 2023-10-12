use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::future::Future;
use std::sync::Arc;
use std::{convert::Infallible, pin::Pin};
use std::{
    net::{SocketAddr, SocketAddrV4},
    str,
};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{self, Duration};
use tracing::{debug, error, trace, warn, Span};

// Encapsulates data types and algorithms related to NTP server
// information.

mod server {
    use super::*;

    // Holds interesting state information for an NTP server.

    #[derive(Debug, PartialEq)]
    pub struct Info(String, f64, f64);

    impl Info {
        // Creates a new, initialized `Info` type.

        pub fn new(host: String, offset: f64, delay: f64) -> Info {
            Info(host, offset, delay)
        }

        // Creates a value which will never match any value returned
        // by an NTP server (because the host will never be blank.)

        pub fn bad_value() -> Info {
            Info(String::from(""), 0.0, 0.0)
        }

        // Returns the IP address of the NTP server.

        pub fn get_host(&self) -> &String {
            &self.0
        }

        // Returns the estimated offset (in milliseconds) of the
        // system time compared to the NTP server.

        pub fn get_offset(&self) -> f64 {
            self.1
        }

        // Returns the estimated time-of-flight delay (in
        // milliseconds) to the NTP server.

        pub fn get_delay(&self) -> f64 {
            self.2
        }
    }

    // Updates the `Info` object using up to three "interesting"
    // parameters from text consisting of comma-separated,
    // key/value pairs. The original `Info` is consumed by this
    // method.

    fn update_host_info(
        mut state: (Option<String>, Option<f64>, Option<f64>),
        item: &str,
    ) -> (Option<String>, Option<f64>, Option<f64>) {
        match item.split('=').collect::<Vec<&str>>()[..] {
            ["srcadr", adr] => state.0 = Some(String::from(adr)),
            ["offset", offset] => {
                if let Ok(o) = offset.parse::<f64>() {
                    state.1 = Some(o)
                }
            }
            ["delay", delay] => {
                if let Ok(d) = delay.parse::<f64>() {
                    state.2 = Some(d)
                }
            }
            _ => (),
        }
        state
    }

    // Returns an `Info` type that has been initialized with the
    // parameters defined in `input`.

    pub fn decode_info(input: &str) -> Option<Info> {
        let result = input
            .split(',')
            .filter(|v| !v.is_empty())
            .map(|v| v.trim_start())
            .fold((None, None, None), update_host_info);

        if let (Some(a), Some(o), Some(d)) = result {
            Some(Info::new(a, o, d))
        } else {
            None
        }
    }
}

pub struct Instance {
    sock: UdpSocket,
    seq: u16,
}

pub struct Devices {
    d_state: driver::ReportReading<bool>,
    d_source: driver::ReportReading<String>,
    d_offset: driver::ReportReading<f64>,
    d_delay: driver::ReportReading<f64>,
}

impl Instance {
    pub const NAME: &'static str = "ntp";

    pub const SUMMARY: &'static str =
        "monitors an NTP server and reports its state";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    // Attempts to pull the hostname/port for the remote process.

    fn get_cfg_address(cfg: &DriverConfig) -> Result<SocketAddrV4> {
        match cfg.get("addr") {
            Some(toml::value::Value::String(addr)) => {
                if let Ok(addr) = addr.parse::<SocketAddrV4>() {
                    Ok(addr)
                } else {
                    Err(Error::ConfigError(String::from(
                        "'addr' not in hostname:port format",
                    )))
                }
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'addr' config parameter should be a string",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'addr' parameter in config",
            ))),
        }
    }

    // Combines and returns the first two bytes from a buffer as a
    // big-endian, 16-bit value.

    fn read_u16(buf: &[u8]) -> u16 {
        (buf[0] as u16) * 256 + (buf[1] as u16)
    }

    async fn get_synced_host(&mut self) -> Option<u16> {
        let req: [u8; 12] = [
            0x26,
            0x01,
            (self.seq / 256) as u8,
            (self.seq % 256) as u8,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
        ];

        self.seq += 1;

        // Try to send the request. If there's a failure with the
        // socket, report the error and return `None`.

        if let Err(e) = self.sock.send(&req).await {
            error!("couldn't send \"synced hosts\" request -> {}", e);
            return None;
        }

        let mut buf = [0u8; 500];

        #[rustfmt::skip]
	tokio::select! {
	    result = self.sock.recv(&mut buf) => {
		match result {
		    // The packet has to be at least 12 bytes so we
		    // can use all parts of the header without
		    // worrying about panicking.

		    Ok(len) if len < 12 => {
			warn!(
			    "response from ntpd < 12 bytes -> only {} bytes",
			    len
			)
		    }

		    Ok(len) => {
			let total = Instance::read_u16(&buf[10..=11]) as usize;
			let expected_len = total + 12 + (4 - total % 4) % 4;

			// Make sure the incoming buffer is as large
			// as the length field says it is (so we can
			// safely access the entire payload.)

			if expected_len == len {
			    for ii in buf[12..len].chunks_exact(4) {
				if (ii[2] & 0x7) == 6 {
				    return Some(Instance::read_u16(
					&ii[0..=1],
				    ));
				}
			    }
			} else {
			    warn!(
				"bad packet length -> expected {}, got {}",
				expected_len, len
			    );
			}
		    }
		    Err(e) => error!("couldn't receive data -> {}", e),
		}
	    },
	    _ = tokio::time::sleep(std::time::Duration::from_millis(1_000)) => {
		warn!("timed-out waiting for reply to \"get synced host\" request")
	    }
	}

        None
    }

    // Requests information about a given association ID. An `Info`
    // type is returned containing the parameters we find interesting.

    pub async fn get_host_info(&mut self, id: u16) -> Option<server::Info> {
        let req = &[
            0x26,
            0x02,
            (self.seq / 256) as u8,
            (self.seq % 256) as u8,
            0x00,
            0x00,
            (id / 256) as u8,
            (id % 256) as u8,
            0x00,
            0x00,
            0x00,
            0x00,
        ];

        self.seq += 1;

        if let Err(e) = self.sock.send(req).await {
            error!("couldn't send \"host info\" request -> {}", e);
            return None;
        }

        let mut buf = [0u8; 500];
        let mut payload = [0u8; 2048];
        let mut next_offset = 0;

        loop {
            #[rustfmt::skip]
	    tokio::select! {
		result = self.sock.recv(&mut buf) => {
		    match result {
			// The packet has to be at least 12 bytes so
			// we can use all parts of the header without
			// worrying about panicking.

			Ok(len) if len < 12 => {
			    warn!("response from ntpd < 12 bytes -> {}", len);
			    break;
			}

			Ok(len) => {
			    let offset = Instance::read_u16(&buf[8..=9]) as usize;

			    // We don't keep track of which of the
			    // multiple packets we've already
			    // received. Instead, we require the
			    // packets are sent in order. This warning
			    // has never been emitted.

			    if offset != next_offset {
				warn!("dropped packet (incorrect offset)");
				break;
			    }

			    let total = Instance::read_u16(&buf[10..=11]) as usize;
			    let expected_len = total + 12 + (4 - total % 4) % 4;

			    // Make sure the incoming buffer is as
			    // large as the length field says it is
			    // (so we can safely access the entire
			    // payload.)

			    if expected_len != len {
				warn!(
				    "bad packet length -> expected {}, got {}",
				    expected_len, len
				);
				break;
			    }

			    // Make sure the reply's offset and total
			    // won't push us past the end of our
			    // buffer.

			    if offset + total > payload.len() {
				warn!(
				    "payload too big (offset {}, total {}, target buf: {})",
				    offset,
				    total,
				    payload.len()
				);
				break;
			    }

			    // Update the next, expected offset.

			    next_offset += total;

			    // Copy the fragment into the final buffer.

			    let dst_range = offset..offset + total;
			    let src_range = 12..12 + total;

			    trace!(
				"copying {} bytes into {} through {}",
				dst_range.len(),
				dst_range.start,
				dst_range.end - 1
			    );

			    payload[dst_range].clone_from_slice(&buf[src_range]);

			    // If this is the last packet, we can
			    // process it. Convert the byte buffer to
			    // text and decode it.

			    if (buf[1] & 0x20) == 0 {
				let payload = &payload[..next_offset];

				return str::from_utf8(payload)
				    .ok()
				    .and_then(server::decode_info)
			    }
			}
			Err(e) => {
			    error!("couldn't receive data -> {}", e);
			    break;
			}
		    }
		},
		_ = tokio::time::sleep(std::time::Duration::from_millis(1_000)) => {
		    warn!("timed-out waiting for reply to \"get host info\" request")
		}
	    }
        }
        None
    }
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan,
        _: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        // It's safe to use `.unwrap()` for these names because, in a
        // fully-tested, released version of this driver, we would
        // have seen and fixed any panics.

        let state_name = "state".parse::<device::Base>().unwrap();
        let source_name = "source".parse::<device::Base>().unwrap();
        let offset_name = "offset".parse::<device::Base>().unwrap();
        let delay_name = "delay".parse::<device::Base>().unwrap();

        Box::pin(async move {
            // Define the devices managed by this driver.

            let (d_state, _) =
                core.add_ro_device(state_name, None, max_history).await?;
            let (d_source, _) =
                core.add_ro_device(source_name, None, max_history).await?;
            let (d_offset, _) = core
                .add_ro_device(offset_name, Some("ms"), max_history)
                .await?;
            let (d_delay, _) = core
                .add_ro_device(delay_name, Some("ms"), max_history)
                .await?;

            Ok(Devices {
                d_state,
                d_source,
                d_offset,
                d_delay,
            })
        })
    }

    fn create_instance(
        cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        let addr = Instance::get_cfg_address(cfg);

        let fut = async move {
            // Validate the configuration.

            let addr = addr?;
            let loc_if = "0.0.0.0:0".parse::<SocketAddr>().unwrap();

            Span::current().record("cfg", addr.to_string());

            if let Ok(sock) = UdpSocket::bind(loc_if).await {
                if sock.connect(addr).await.is_ok() {
                    return Ok(Box::new(Instance { sock, seq: 1 }));
                }
            }
            Err(Error::OperationError("couldn't create socket".to_owned()))
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Devices>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            // Record the peer's address in the "cfg" field of the
            // span.

            {
                let addr = self
                    .sock
                    .peer_addr()
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|_| String::from("**unknown**"));

                Span::current().record("cfg", addr.as_str());
            }

            // Set `info` to an initial, unmatchable value. `None`
            // would be preferrable here but, if DrMem had a problem
            // at startup getting the NTP state, it wouldn't print the
            // warning(s).

            let mut info = Some(server::Info::bad_value());
            let mut interval = time::interval(Duration::from_millis(20_000));

            let devices = devices.lock().await;

            loop {
                interval.tick().await;

                if let Some(id) = self.get_synced_host().await {
                    debug!("synced to host ID: {:#04x}", id);

                    let host_info = self.get_host_info(id).await;

                    match host_info {
                        Some(ref tmp) => {
                            if info != host_info {
                                debug!(
                                    "host: {}, offset: {} ms, delay: {} ms",
                                    tmp.get_host(),
                                    tmp.get_offset(),
                                    tmp.get_delay()
                                );
                                (devices.d_source)(tmp.get_host().clone())
                                    .await;
                                (devices.d_offset)(tmp.get_offset()).await;
                                (devices.d_delay)(tmp.get_delay()).await;
                                (devices.d_state)(true).await;
                                info = host_info;
                            }
                            continue;
                        }
                        None => {
                            if info.is_some() {
                                warn!("no synced host information found");
                                info = None;
                                (devices.d_state)(false).await;
                            }
                        }
                    }
                } else if info.is_some() {
                    warn!("we're not synced to any host");
                    info = None;
                    (devices.d_state)(false).await;
                }
            }
        };

        Box::pin(fut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoding() {
        assert_eq!(
            server::decode_info("srcadr=192.168.1.1,offset=0.0,delay=0.0"),
            Some(server::Info::new(String::from("192.168.1.1"), 0.0, 0.0))
        );
        assert_eq!(
            server::decode_info(" srcadr=192.168.1.1, offset=0.0, delay=0.0"),
            Some(server::Info::new(String::from("192.168.1.1"), 0.0, 0.0))
        );

        // Should return `None` if fields are missing.

        assert_eq!(server::decode_info(" offset=0.0, delay=0.0"), None);
        assert_eq!(server::decode_info(" srcadr=192.168.1.1, delay=0.0"), None);
        assert_eq!(
            server::decode_info(" srcadr=192.168.1.1, offset=0.0"),
            None
        );

        // Test badly formed input.

        assert!(server::decode_info("srcadr=192.168.1.1,offset=b,delay=0.0")
            .is_none());
        assert!(server::decode_info("srcadr=192.168.1.1,offset=0.0,delay=b")
            .is_none());
    }
}
