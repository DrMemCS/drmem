// Copyright (c) 2020-2021, Richard M Neswold, Jr.
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
use toml::value;

pub mod types;
pub mod device;

/// A `Result` type where the error value is a value from
/// `drmem_api::types::Error`.

pub type Result<T> = std::result::Result<T, types::Error>;

/// The `DbContext` trait defines the API that a back-end needs to
/// implement to provide storage for -- and access to -- the state of
/// each driver's devices.

#[async_trait]
pub trait DbContext {
    /// This associated type defines the structure of configuration
    /// information.
    type Cfg;

    /// Creates an instance of the `Context`.
    async fn create(name: &str, cfg: &Self::Cfg, account: Option<String>,
		    password: Option<String>) -> Result<Box<Self>>;

    /// Used by a driver to define a readable device. `name` specifies
    /// the final segment of the device name (the prefix is determined
    /// by the driver's name.) `summary` should be a one-line
    /// description of the device. `units` is an optional units
    /// field. Some devices (like boolean or string devices) don't
    /// require engineering units.
    async fn define_device<T: types::Compat + Send>(&mut self,
						    name: &str,
						    summary: &str,
						    units: Option<String>) ->
	Result<device::Device<T>>;

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
    async fn write_values(&mut self, values: &[(String, types::DeviceValue)])
			  -> Result<()>;
}

/// All drivers implement the `Driver` trait.
#[async_trait]
pub trait Driver {

    /// Creates a new instance of the driver. `ctxt` will contain the
    /// driver's connection with the backend storage. `cfg` is a
    /// HashMap table containing configuration information. This
    /// information is obtained from the TOML configuration file. Each
    /// driver has its own configuration information. It is
    /// recommended that the driver validate the configuration.
    async fn new(ctxt: impl DbContext, addr: value::Table) -> Self;

    /// The name of the driver. This should be relatively short, but
    /// needs to be unique across all drivers.
    fn name() -> String;

    /// A detailed description of the driver. The format of the string
    /// should be markdown. The description should include any
    /// configuration parameter needed in the TOML configuration
    /// file. It should also mention the endpoints provided by the
    /// driver.
    fn description() -> String;

    /// A short, one-line summary of the driver.
    fn summary(&self) -> String;
}
