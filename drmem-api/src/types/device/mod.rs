//! Defines types related to devices.

use std::{pin::Pin, time};
use tokio_stream::Stream;

mod value;
pub use value::Value;

/// Represents the value of a device at a specific moment.
///
/// When a client monitors a device, it receives a stream of readings
/// as the device gets updated. A reading consists of the value of the
/// device along with the timestamp. The set of types that a device
/// can return is defined in the `Value` type. The timestamp is given
/// in UTC.
#[derive(Debug, PartialEq, Clone)]
pub struct Reading {
    pub ts: time::SystemTime,
    pub value: Value,
}

/// Generic type describing a stream of types.
///
/// Specializations of this type are used in various layers of
/// `drmemd`. The drivers, for instance, provide a stream of `Reading`
/// types. The GraphQL layer converts it into a stream of replies.
pub type DataStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

mod name;
pub use name::Base;
pub use name::Name;
pub use name::Path;
