#[cfg(feature = "simple-backend")]
pub mod simple;
#[cfg(feature = "simple-backend")]
pub use simple as store;

#[cfg(feature = "redis-backend")]
pub mod redis;
#[cfg(feature = "redis-backend")]
pub use redis as store;
