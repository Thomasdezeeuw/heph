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
//! standard out. To log a request the `request` macro can be used, which is
//! re-exported from the `std-logger` crate.
//!
//! To enable logging call [`init`].
//!
//! [`log`]: https://crates.io/crates/log
//!
//! # Examples
//!
//! Enabling logging.
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::supervisor::NoSupervisor;
//! use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};
//! use heph::log::{self, request};
//!
//! fn main() -> Result<(), rt::Error> {
//!     // Enable logging.
//!     log::init();
//!
//!     Runtime::new()?.with_setup(add_greeter_actor).start()
//! }
//!
//! fn add_greeter_actor(mut system_ref: RuntimeRef) -> Result<(), !> {
//!     let actor = greeter_actor as fn(_) -> _;
//!     system_ref.spawn_local(NoSupervisor, actor, (), ActorOptions::default().mark_ready());
//!     Ok(())
//! }
//!
//! async fn greeter_actor(_: actor::Context<!>) -> Result<(), !> {
//!     // Log a request.
//!     request!("Hello world");
//!
//!     Ok(())
//! }
//! ```
//!
//! [`std-logger`]: std_logger
// [`request`]: crate::log::request
// FIXME: this doesn't seem to work, see Rust issue #43466.
//! [`init`]: crate::log::init

#[doc(no_inline)]
pub use log::{debug, error, info, log, log_enabled, trace, warn};

#[doc(no_inline)]
pub use std_logger::{init, request, try_init, REQUEST_TARGET};
