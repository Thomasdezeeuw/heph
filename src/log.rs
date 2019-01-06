//! Logging related types.
//!
//! Logging in Heph is done via the [`log`] crate, much like the entire Rust
//! ecosystem does (or should). However the log crate doesn't provide an actual
//! logging implementation, it only defines macros for it. Those macros are
//! re-exported here, which means that the macros in the `log` crate can also be
//! used.
//!
//! The actual logging implementation comes from the [`std-logger`] crate. By
//! default logs are written to standard error and requests log are written to
//! standard out. To mark a log message as a request message the
//! `REQUEST_TARGET` constant can be used, which is re-exported from the
//! `std-logger` crate.
//!
//! To enable logging use the [`enable_logging`] method on the `ActorSystem`
//! struct, or by calling [`init`].
//!
//! [`log`]: ../../log/index.html
//! [`std-logger`]: ../../std_logger/index.html
//! [`enable_logging`]: ../system/struct.ActorSystem.html#method.enable_logging
//! [`init`]: fn.init.html

#[doc(no_inline)]
pub use log::{debug, error, info, log, log_enabled, trace, warn};

#[doc(no_inline)]
pub use std_logger::REQUEST_TARGET;

/// Initialise logging.
///
/// This is whats get called when using [`ActorSystem.enable_logging`]. This is
/// made public here so it can be called before the [`ActorSystem`] is created
/// and ran.
///
/// [`ActorSystem.enable_logging`]: ../system/struct.ActorSystem.html#method.enable_logging
/// [`ActorSystem`]: ../system/struct.ActorSystem.html
pub fn init() {
    std_logger::init();
}
