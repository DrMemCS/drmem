//!This module defines types related to devices.
use std::{pin::Pin, time};
use tokio_stream::Stream;

mod value;
pub use value::Value;

#[derive(Debug, PartialEq, Clone)]
pub struct Reading {
    pub ts: time::SystemTime,
    pub value: Value,
}

pub type DataStream<T> =
    Pin<Box<dyn Stream<Item = T> + Send + Sync>>;

mod name;
pub use name::Base;
pub use name::Name;
pub use name::Path;
