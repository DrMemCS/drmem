use drmem_api::{
    driver::{self, DriverConfig},
    types::{device::Base, Error},
    Result,
};
use std::future::Future;
use std::{convert::Infallible, pin::Pin};
use std::{net::{SocketAddr, SocketAddrV4}, str};
use tokio::net::UdpSocket;
use tokio::time::{self, Duration};
use tracing::{debug, error, info, warn, Span};

// Encapsulates data types and algorithms related to NTP server
// information.

mod server {
    use super::*;

    // Holds interesting state information for an NTP server.

    #[derive(PartialEq)]
    pub struct Info((String, f64, f64));

    impl Info {
        // Creates a new, initialized `Info` type.

        pub fn new() -> Info {
            Info((String::from(""), 0.0, 0.0))
        }

        // Returns the IP address of the NTP server.

        pub fn get_host(&self) -> &String {
            &self.0 .0
        }

        // Returns the estimated offset (in milliseconds) of the
        // system time compared to the NTP server.

        pub fn get_offset(&self) -> f64 {
            self.0 .1
        }

        // Returns the estimated time-of-flight delay (in
        // milliseconds) to the NTP server.

        pub fn get_delay(&self) -> f64 {
            self.0 .2
        }

        // Updates the `Info` object using up to three "interesting"
        // parameters from text consisting of comma-separated,
        // key/value pairs. The original `Info` is comsumed by this
        // method.

        fn update_host_info(mut self, item: &str) -> Info {
            match item.split('=').collect::<Vec<&str>>()[..] {
                ["srcadr", adr] => self.0 .0 = String::from(adr),
                ["offset", offset] => {
                    self.0 .1 = offset.parse::<f64>().unwrap()
                }
                ["delay", delay] => self.0 .2 = delay.parse::<f64>().unwrap(),
                _ => (),
            }
            self
        }
    }

    impl Default for Info {
        fn default() -> Self {
            Self::new()
        }
    }

    // Returns an `Info` type that has been initialized with the
    // parameters defined in `input`.

    pub fn decode_info(input: &str) -> Info {
        input
            .split(',')
            .filter(|v| !v.is_empty())
            .map(|v| v.trim_start())
            .fold(Info::new(), Info::update_host_info)
    }
}

pub struct Instance {
    sock: UdpSocket,
    seq: u16,
    d_state: driver::ReportReading,
    d_source: driver::ReportReading,
    d_offset: driver::ReportReading,
    d_delay: driver::ReportReading,
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
                    return Ok(addr);
                } else {
                    error!("'addr' not in hostname:port format")
                }
            }
            Some(_) => error!("'addr' config parameter should be a string"),
            None => error!("missing 'addr' parameter in config"),
        }

        Err(Error::BadConfig)
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

        match self.sock.send(&req).await {
            Ok(_) => {
                let mut buf = [0u8; 500];

                match self.sock.recv(&mut buf).await {
                    // The packet has to be at least 12 bytes so we can
                    // use all parts of the header without worrying about
                    // panicking.
                    Ok(len) if len < 12 => {
                        warn!("response from ntpd < 12 bytes -> {}", len)
                    }

                    Ok(len) => {
                        let total = Instance::read_u16(&buf[10..=11]) as usize;
                        let expected_len = total + 12 + (4 - total % 4) % 4;

                        // Make sure the incoming buffer is as large as
                        // the length field says it is (so we can safely
                        // access the entire payload.)

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
            }
            Err(e) => error!("couldn't send request -> {}", e),
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
            error!("couldn't send request -> {}", e)
        } else {
            let mut buf = [0u8; 500];
            let mut payload = [0u8; 2048];
            let mut next_offset = 0;

            loop {
                match self.sock.recv(&mut buf).await {
                    // The packet has to be at least 12 bytes so we
                    // can use all parts of the header without
                    // worrying about panicking.
                    Ok(len) if len < 12 => {
                        warn!("response from ntpd < 12 bytes -> {}", len);
                        break;
                    }

                    Ok(len) => {
                        let offset = Instance::read_u16(&buf[8..=9]) as usize;

                        // We don't keep track of which of the
                        // multiple packets we've already received.
                        // Instead, we require the packets are sent in
                        // order. This warning has never been emitted.

                        if offset != next_offset {
                            warn!("dropped packet (incorrect offset)");
                            break;
                        }

                        let total = Instance::read_u16(&buf[10..=11]) as usize;
                        let expected_len = total + 12 + (4 - total % 4) % 4;

                        // Make sure the incoming buffer is as large
                        // as the length field says it is (so we can
                        // safely access the entire payload.)

                        if expected_len != len {
                            warn!(
                                "bad packet length -> expected {}, got {}",
                                expected_len, len
                            );
                            break;
                        }

                        // Make sure the reply's offset and total
                        // won't push us past the end of our buffer.

                        if offset + total > payload.len() {
                            warn!(
                                "payload too big (offset {}, total {}
                                 , target buf: {}",
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

                        debug!(
                            "copying {} bytes into {} through {}",
                            dst_range.len(),
                            dst_range.start,
                            dst_range.end - 1
                        );

                        payload[dst_range].clone_from_slice(&buf[src_range]);

                        // If this is the last packet, we can process
                        // it. Convert the byte buffer to text and
                        // decode it.

                        if (buf[1] & 0x20) == 0 {
                            let payload = &payload[..next_offset];

                            return str::from_utf8(payload)
                                .map(server::decode_info)
                                .ok();
                        }
                    }
                    Err(e) => {
                        error!("couldn't receive data -> {}", e);
                        break;
                    }
                }
            }
        }
        None
    }
}

impl driver::API for Instance {
    fn create_instance(
        cfg: DriverConfig, core: driver::RequestChan,
    ) -> Pin<
        Box<dyn Future<Output = Result<driver::DriverType>> + Send + 'static>,
    > {
        let fut = async move {
            // Validate the configuration.

            let addr = Instance::get_cfg_address(&cfg)?;
            let loc_if = "0.0.0.0:0".parse::<SocketAddr>().unwrap();

            if let Ok(sock) = UdpSocket::bind(loc_if).await {
                if sock.connect(addr).await.is_ok() {
                    // Define the devices managed by this driver.

                    let (d_state, _) = core
                        .add_ro_device("state".parse::<Base>()?, None)
                        .await?;
                    let (d_source, _) = core
                        .add_ro_device("source".parse::<Base>()?, None)
                        .await?;
                    let (d_offset, _) = core
                        .add_ro_device("offset".parse::<Base>()?, Some("ms"))
                        .await?;
                    let (d_delay, _) = core
                        .add_ro_device("delay".parse::<Base>()?, Some("ms"))
                        .await?;

                    return Ok(Box::new(Instance {
                        sock,
                        seq: 1,
                        d_state,
                        d_source,
                        d_offset,
                        d_delay,
                    }) as driver::DriverType);
                }
            }
            Err(Error::OperationError)
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Infallible>> + Send + 'a>> {
        let fut = async {
            // Record the peer's address in the "cfg" field of the
            // span.

            {
                let addr = self
                    .sock
                    .peer_addr()
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|_| String::from("**unknown**"));

                Span::current().record("cfg", &addr.as_str());
            }

            let mut info = server::Info::new();
            let mut warning_printed = false;
            let mut interval = time::interval(Duration::from_millis(20_000));

            loop {
                interval.tick().await;

                if let Some(id) = self.get_synced_host().await {
                    debug!("synced to host ID: {:#04x}", id);

                    if let Some(inf) = self.get_host_info(id).await {
                        if inf != info {
                            info!(
                                "host: {}, offset: {} ms, delay: {} ms",
                                inf.get_host(),
                                inf.get_offset(),
                                inf.get_delay()
                            );
                            info = inf;
                            warning_printed = false;
                            (self.d_source)(info.get_host().into()).await?;
                            (self.d_offset)(info.get_offset().into()).await?;
                            (self.d_delay)(info.get_delay().into()).await?;
                            (self.d_state)(true.into()).await?;
                        }
                    } else if !warning_printed {
                        warn!("no synced host information found");
                        warning_printed = true;
                        (self.d_state)(false.into()).await?;
                    }
                } else if !warning_printed {
                    warn!("we're not synced to any host");
                    warning_printed = true;
                    (self.d_state)(false.into()).await?;
                }
            }
        };

        Box::pin(fut)
    }
}

#[cfg(test)]
mod tests {}
