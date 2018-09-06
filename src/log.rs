//! Logging related types.
//!
//! Logging in Heph is done via the [`log`] crate, must like the entire Rust
//! ecosystem does (or should). However the log crate doesn't provide an actual
//! logging implementation, it only defines macros for it. Those macros are
//! re-exported here, which means that the macros in the log crate can also be
//! used.
//!
//! The actual logging implementation comes from the [`std-logger`] crate. By
//! default logs are written to standard error and requests log are written to
//! standard out. To mark a log messages as a request log the `REQUEST_TARGET`
//! constant can be used, which is re-exported from the `std-logger` crate.
//!
//! To enable logging use the [`enable_logging`] method on the `ActorSystem`
//! struct.
//!
//! [`log`]: ../../log/index.html
//! [`std-logger`]: ../../std_logger/index.html
//! [`enable_logging`]: ../system/struct.ActorSystem.html#method.enable_logging

pub use log::{debug, error, info, log, log_enabled, trace, warn};

pub use std_logger::REQUEST_TARGET;
