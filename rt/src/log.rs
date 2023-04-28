//! Logging.
//!
//! Logging in Heph is done via the [`log`] crate, much like the entire Rust
//! ecosystem does (or should). However the log crate doesn't provide an actual
//! logging implementation, it only defines macros for logging.
//!
//! Heph doesn't provide a logging implementation, but it recommends the
//! [`std-logger`] crate.
//!
//! [`log`]: https://crates.io/crates/log
//! [`std-logger`]: https://crates.io/crates/std_logger
//!
//! # Examples
//!
//! Enabling logging.
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph_rt::Runtime;
//! use log::info;
//!
//! fn main() -> Result<(), heph_rt::Error> {
//!     // Enable logging.
//!     std_logger::Config::logfmt().init();
//!
//!     let runtime = Runtime::new()?;
//!     // Runtime setup etc.
//!     info!("starting runtime");
//!     runtime.start()
//! }
//! ```
