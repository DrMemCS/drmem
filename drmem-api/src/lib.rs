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

use async_trait::async_trait;
use drmem_types::{DeviceValue, Error};

pub mod device;

/// A `Result` type where the error value is a value from
/// `drmem_api::types::Error`.

pub type Result<T> = std::result::Result<T, Error>;

/// The `DbContext` trait defines the API that a back-end needs to
/// implement to provide storage for -- and access to -- the state of
/// each driver's devices.

#[async_trait]
pub trait DbContext {
    /// Used by a driver to define a readable device. `name` specifies
    /// the final segment of the device name (the prefix is determined
    /// by the driver's name.) `summary` should be a one-line
    /// description of the device. `units` is an optional units
    /// field. Some devices (like boolean or string devices) don't
    /// require engineering units.
    async fn define_device<T: Into<DeviceValue> + Send>(
        &mut self, name: &str, summary: &str, units: Option<String>,
    ) -> Result<device::Device<T>>;

    /// Allows a driver to write values, associated with devices, to
    /// the database. The `values` array indicate which devices should
    /// be updated.
    ///
    /// If multiple devices change simultaneously (e.g. a device's
    /// value is computed from other devices), a driver is strongly
    /// recommended to make a single call with all the affected
    /// devices. Each call to this function makes an atomic change to
    /// the database so if all devices are changed in a single call,
    /// clients will see a consistent change.
    async fn write_values(
        &mut self, values: &[(String, DeviceValue)],
    ) -> Result<()>;
}

pub mod client {
    use tokio::sync::{mpsc, oneshot};

    enum Request {
        Ping(oneshot::Sender<()>),
        GetSummary(oneshot::Sender<&'static str>),
        GetDescription(oneshot::Sender<&'static str>),
    }

    pub struct RequestChan(mpsc::Sender<Request>);
}

pub mod driver;
