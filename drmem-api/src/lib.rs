//! This crate is used by hardware drivers.
//!
//! The interfaces and types defined in this crate are useful for
//! those wishing to write a new driver for the `drmemd` executable.

mod types;

// Pull types down to the `drmem-api` namespace.

pub use types::device;
pub use types::Error;

/// A specialization of `std::result::Result<>` where the error value
/// is `types::Error`.

pub type Result<T> = std::result::Result<T, Error>;

pub mod client;
pub mod driver;
