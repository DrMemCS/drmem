// Copyright (c) 2020-2022, Richard M Neswold, Jr.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
// 1. Redistributions of source code must retain the above copyright
//    notice, this list of conditions and the following disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright
//    notice, this list of conditions and the following disclaimer in the
//    documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its
//    contributors may be used to endorse or promote products derived
//    from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! This module defines types and interfaces that driver use to
//! interact with the core of DrMem.

use async_trait::async_trait;
use drmem_types::{DeviceValue, Error};
use tokio::sync::{broadcast, mpsc, oneshot};
use toml::value;

use super::Result;

pub type Config = value::Table;
pub type TxDeviceValue = broadcast::Sender<DeviceValue>;
pub type TxDeviceSetting = mpsc::Sender<DeviceValue>;
pub type RxDeviceSetting = mpsc::Receiver<DeviceValue>;

/// Defines the requests that can be sent to core.
#[derive(Debug)]
pub enum Request {
    /// Registers a read-only device with core.
    ///
    /// The reply will contain a channel to broadcast values read from
    /// the hardware.
    AddReadonlyDevice {
        dev_name: String,
        rpy_chan: oneshot::Sender<Result<TxDeviceValue>>,
    },

    /// Registers a writable device with core.
    ///
    /// The reply is a pair where the first element is a channel to
    /// broadcast values read from the hardware. The second element is
    /// a read-handle to acccept incoming setting to the device.
    AddReadWriteDevice {
        dev_name: String,
        rpy_chan: oneshot::Sender<Result<(TxDeviceValue, RxDeviceSetting)>>,
    },
}

pub type Reading<T> = Box<
    dyn Fn(
        T,
    ) -> std::result::Result<
        usize,
        broadcast::error::SendError<DeviceValue>,
    >,
>;

/// A handle which is used to communicate with the core of DrMem.
/// When a driver is created, it will be given a handle to be used
/// throughout its life.
///
/// This type wraps the `mpsc::Sender<>` and defines a set of helper
/// methods to send requests and receive replies with the core.
pub struct RequestChan {
    prefix: String,
    req_chan: mpsc::Sender<Request>,
}

impl RequestChan {
    pub fn new(prefix: &str, req_chan: &mpsc::Sender<Request>) -> Self {
        RequestChan {
            prefix: String::from(prefix),
            req_chan: req_chan.clone(),
        }
    }

    /// Registers a read-only device with the framework. `name` is the
    /// last section of the full device name. Typically a driver will
    /// register several devices, each representing a portion of the
    /// hardware being controlled. All devices for a given driver
    /// instance will have the same prefix; the `name` parameter is
    /// appended to it.
    ///
    /// If it returns `Ok()`, the value is a broadcast channel that
    /// the driver uses to announce new values of the associated
    /// hardware.
    ///
    /// If it returns `Err()`, the underlying value could be `InUse`,
    /// meaning the device name is already registered. If the error is
    /// `InternalError`, then the core has exited and the
    /// `RequestChan` has been closed. Since the driver can't report
    /// any more updates, it may as well shutdown.
    pub async fn add_ro_device<T: Into<DeviceValue>>(
        &self, name: &str,
    ) -> super::Result<Reading<T>> {
        // Create a location for the reply.

        let (tx, rx) = oneshot::channel();

        // Send a request to Core to register the given name.
        //
        // XXX: Device names should be handled more formally. This
        // code doesn't check that the names are of the correct
        // character set.

        let result = self
            .req_chan
            .send(Request::AddReadonlyDevice {
                dev_name: format!("{}:{}", self.prefix, name),
                rpy_chan: tx,
            })
            .await;

        // If the request was sent successfully and we successfully
        // received a reply, process the payload.

        if result.is_ok() {
            match rx.await {
                // If the name was successfully registered, core will
                // return a sending handle for device readings. Wrap
                // the handle in a type-safe wrapper (so the driver
                // can only send the correct type.)
                Ok(Ok(ch)) => {
                    return Ok(Box::new(move |v: T| ch.send(v.into())))
                }

                Ok(Err(e)) => return Err(e),

                Err(_) => (),
            }
        }

        // If either communication direction failed, return an error
        // indicating we can't talk to core.

        Err(Error::MissingPeer(String::from("core")))
    }

    /// Registers a read-write device with the framework. `name` is the
    /// last section of the full device name. Typically a driver will
    /// register several devices, each representing a portion of the
    /// hardware being controlled. All devices for a given driver
    /// instance will have the same prefix; the `name` parameter is
    /// appended to it.
    ///
    /// If it returns `Ok()`, the value is a pair containing a
    /// broadcast channel that the driver uses to announce new values
    /// of the associated hardware and a receive channel for incoming
    /// settings to be applied to the hardware.
    ///
    /// If it returns `Err()`, the underlying value could be `InUse`,
    /// meaning the device name is already registered. If the error is
    /// `InternalError`, then the core has exited and the
    /// `RequestChan` has been closed. Since the driver can't report
    /// any more updates or accept new settings, it may as well shutdown.
    pub async fn add_rw_device(
        &self, name: &str,
    ) -> Result<(broadcast::Sender<DeviceValue>, mpsc::Receiver<DeviceValue>)>
    {
        let (tx, rx) = oneshot::channel();
        let result = self
            .req_chan
            .send(Request::AddReadWriteDevice {
                dev_name: format!("{}:{}", self.prefix, name),
                rpy_chan: tx,
            })
            .await;

        if result.is_ok() {
            if let Ok(v) = rx.await {
                return v;
            }
        }
        Err(Error::MissingPeer(String::from("core")))
    }
}

/// All drivers implement the `driver::API` trait.
#[async_trait]
pub trait API {
    fn create(cfg: Config, drc: RequestChan) -> Result<Box<dyn API>>
    where
        Self: Sized;

    async fn run(mut self) -> Result<()>;

    /// The name of the driver. This should be relatively short, but
    /// needs to be unique across all drivers.
    fn name(&self) -> &'static str;

    /// A detailed description of the driver. The format of the string
    /// should be markdown. The description should include any
    /// configuration parameter needed in the TOML configuration
    /// file. It should also mention the endpoints provided by the
    /// driver.
    fn description(&self) -> &'static str;

    /// A short, one-line summary of the driver.
    fn summary(&self) -> &'static str;
}
