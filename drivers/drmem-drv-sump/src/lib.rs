// Defines the configuration parameters for the driver.
mod config;

// Defines the device set that each instance of the driver manages.
mod device;

// The actual driver code.
mod driver;

pub use driver::Instance;
