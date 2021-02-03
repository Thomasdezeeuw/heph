//! Logging related types.
//!
//! Logging in Heph is done via the [`log`] crate, much like the entire Rust
//! ecosystem does (or should). However the log crate doesn't provide an actual
//! logging implementation, it only defines macros for it. Those macros are
//! re-exported here, which means that the macros in the `log` crate can also be
//! used.
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
//! use heph::supervisor::NoSupervisor;
//! use heph::{actor, rt, ActorOptions, Runtime, RuntimeRef};
//! use log::info;
//!
//! fn main() -> Result<(), rt::Error> {
//!     // Enable logging.
//!     std_logger::init();
//!
//!     Runtime::new()?.with_setup(add_greeter_actor).start()
//! }
//!
//! fn add_greeter_actor(mut system_ref: RuntimeRef) -> Result<(), !> {
//!     let actor = greeter_actor as fn(_) -> _;
//!     system_ref.spawn_local(NoSupervisor, actor, (), ActorOptions::default());
//!     Ok(())
//! }
//!
//! async fn greeter_actor(_: actor::Context<!>) -> Result<(), !> {
//!     // Log an informational message.
//!     info!("Hello world");
//!
//!     Ok(())
//! }
//! ```

#[doc(no_inline)]
pub use log::{debug, error, info, log, log_enabled, trace, warn};
