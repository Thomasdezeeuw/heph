//! Process signals and handling.
//!
//! # Signal Handling
//!
//! All actors can receive process signals by calling one of the following
//! functions using their actor reference. This causes all process signals to be
//! relayed to the actor which should handle them accordingly.
//!  * [`Runtime::receive_signals`]
//!  * [`RuntimeRef::receive_signals`]
//!
//! Using this method of message passing of the process signals it's easy(-er)
//! to implement graceful shutdown. Any actor that "produces" work, e.g. actors
//! that accept incoming connections, should also listen for incoming process
//! signals. Once a signal arrives that indicate the process should exit, the
//! actor should stop "producing" work. Once the existing work is finish, e.g.
//! already accepted connection handled, the runtime will shut down cleanly as
//! all actors should have stopped.
//!
//! A simple example of receiving a process signal:
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
//!
//! ## Notes
//!
//! What happens to threads spawned outside of Heph's control, i.e. manually
//! spawned, before calling [`rt::Setup::build`] is unspecified. They may still
//! receive a process signal or they may not. This is due to OS limitations and
//! differences. Any manually spawned threads spawned after calling build should
//! not get a process signal.
//!
//! The runtime will only attempt to send the process signal to the actor once.
//! If the message can't be send it's **not** retried. Ensure that the inbox of
//! the actor has enough room to receive the message.
//!
//! [`rt::Setup::build`]: crate::Setup::build
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
#[doc(inline)]
pub use a10::process::{ChildStatus, Signal, WaitId, WaitInfo, WaitOn, WaitOption, wait, wait_on};
