//! Define classes of devices.
//!
//! This module provides a set of types that define a consistent set
//! of device names and channels for classes of hardware devices. For
//! instance, drivers should use the `ColorLight` type if it controls
//! color, LED bulbs. This type will define the set of DrMem devices
//! that are expected from every color LED bulb.

// Pull in the modules that define each hardware type.

mod dimmer_type;
mod switch_type;
mod weather_type;

// Make top-level types available to driver writers.

pub use dimmer_type::Dimmer;
pub use switch_type::Switch;
pub use weather_type::{Units, Weather};
