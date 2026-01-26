//! Process signals and handling.
//!
//! # Signal Handling
//!
//! Signal handling is done via passing messages to actors. See
//!  * [`Runtime::receive_signals`]
//!  * [`RuntimeRef::receive_signals`]
//!
//! A simple example:
//!
//! ```
//! # #![feature(never_type)]
//! use heph::actor;
//! use heph_rt::process::Signal;
//!
//! async fn actor(mut ctx: actor::Context<Signal, ThreadLocal>) -> Result<(), !> {
//!     // Setup receiving of signals send to this process.
//!     let self_ref = ctx.actor_ref();
//!     ctx.runtime().receive_signals(self_ref);
//!
//!     // Now process signals are send as messages.
//!     if let Ok(signal) = ctx.receive_next() {
//!         println!("got a signal: {signal:?}");
//!     }
//! }
//! ```
//!
//! [`Runtime::receive_signals`]: crate::Runtime::receive_signals
//! [`RuntimeRef::receive_signals`]: crate::RuntimeRef::receive_signals
//!
//! # Waiting on Spawned Processess
//!
//! Spawn processes can be waited on using [`wait`] or [`wait_on`]. For example:
//!
//! ```
//! # #![feature(never_type)]
//! use std::io;
//! use std::process::Command;
//!
//! use heph::actor;
//! use heph_rt::process::{self, WaitOption};
//!
//! async fn actor(mut ctx: actor::Context<Signal, ThreadLocal>) -> io::Result<()> {
//!     // Spawn a new process using the standard library.
//!     let child = Command::new("echo").arg("Hello world").spawn()?;
//!     // Wait until it exited (stopped).
//!     let info = process::wait_on(ctx.runtime().sq(), &child).flags(WaitOption::EXITED).await?;
//!     println!("process exited: {info:?}");
//! }
//! ```

// NOTE: not exporting Signals and related types as we already set that up.
pub use a10::process::{ChildStatus, Signal, WaitId, WaitInfo, WaitOn, WaitOption, wait, wait_on};
