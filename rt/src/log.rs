//! Logging related types.
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
//! use heph::actor;
//! use heph::supervisor::NoSupervisor;
//! use heph_rt::spawn::ActorOptions;
//! use heph_rt::{self as rt, Runtime, RuntimeRef, ThreadLocal};
//! use log::info;
//!
//! fn main() -> Result<(), rt::Error> {
//!     // Enable logging.
//!     std_logger::init();
//!
//!     let mut runtime = Runtime::new()?;
//!     runtime.run_on_workers(add_greeter_actor)?;
//!     runtime.start()
//! }
//!
//! fn add_greeter_actor(mut system_ref: RuntimeRef) -> Result<(), !> {
//!     let actor = greeter_actor as fn(_) -> _;
//!     system_ref.spawn_local(NoSupervisor, actor, (), ActorOptions::default());
//!     Ok(())
//! }
//!
//! async fn greeter_actor(_: actor::Context<!, ThreadLocal>) {
//!     // Log an informational message.
//!     info!("Hello world");
//! }
//! ```
