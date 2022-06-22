//!This module defines types related to devices.
use std::time;

mod value;
pub use value::Value;

#[derive(Debug, PartialEq, Clone)]
pub struct Reading {
    pub ts: time::SystemTime,
    pub value: Value,
}

mod name;
pub use name::Base;
pub use name::Name;
pub use name::Path;
