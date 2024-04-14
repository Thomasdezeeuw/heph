//! Heph, derived from [Hephaestus], is the Greek god of blacksmiths,
//! metalworking, carpenters, craftsmen, artisans, sculptors, metallurgy, fire,
//! and volcanoes. Well, this crate has very little to do with Greek gods, but I
//! needed a name.
//!
//! [Hephaestus]: https://en.wikipedia.org/wiki/Hephaestus
//!
//! ## About
//!
//! Heph is an [actor] library based on asynchronous functions. Such
//! asynchronous functions look like this:
//!
//! ```
//! use heph::actor;
//!
//! async fn actor(mut ctx: actor::Context<String>) {
//!     // Receive a message.
//!     if let Ok(msg) = ctx.receive_next().await {
//!         // Print the message.
//!         println!("got a message: {msg}");
//!     }
//! }
//! # _ = actor; // Silence dead code warnings.
//! ```
//!
//! This simple example shows two properties of Heph:
//!  * it's asynchronous nature using `async fn`,
//!  * communication between actors by sending messages, which follows the [actor model].
//!
//! [actor]: https://en.wikipedia.org/wiki/Actor_model
//! [actor model]: https://en.wikipedia.org/wiki/Actor_model
//!
//! ## Getting started
//!
//! There are two ways to get starting with Heph. If you like to see examples,
//! take a look at the [examples] in the examples directory of the source code.
//! If you like to learn more about some of the core concepts of Heph start with
//! the [Quick Start Guide].
//!
//! [examples]: https://github.com/Thomasdezeeuw/heph/blob/main/examples
//! [Quick Start Guide]: crate::quick_start
//!
//! ## Features
//!
//! This crate has one optional feature: `test`. The `test` feature will enable
//! the `test` module which contains testing facilities.

#![feature(const_option, doc_auto_cfg, doc_cfg_hide, never_type)]
#![warn(
    anonymous_parameters,
    bare_trait_objects,
    missing_debug_implementations,
    missing_docs,
    rust_2018_idioms,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results,
    variant_size_differences
)]
// Disallow warnings when running tests.
#![cfg_attr(test, deny(warnings))]
// Disallow warnings in examples, we want to set a good example after all.
#![doc(test(attr(deny(warnings))))]
// The `cfg(any(test, feature = "test"))` attribute creates a doc element
// staying that it's only supporting "using test or test", that is a bit
// confusing. So we hide those parts and instead manually replace all of them
// with: `doc(cfg(feature = "test"))`. That will stay it's only supported using
// the test feature.
#![doc(cfg_hide(any(test, feature = "test")))]

pub mod actor;
pub mod actor_ref;
pub mod future;
pub mod messages;
pub mod quick_start;
pub mod supervisor;
pub mod sync;
#[cfg(any(test, feature = "test"))]
pub mod test;

#[doc(no_inline)]
pub use actor::{actor_fn, Actor, NewActor};
#[doc(no_inline)]
pub use actor_ref::ActorRef;
#[doc(no_inline)]
pub use future::{ActorFuture, ActorFutureBuilder};
#[doc(no_inline)]
pub use supervisor::{Supervisor, SupervisorStrategy, SyncSupervisor};
#[doc(no_inline)]
pub use sync::{SyncActor, SyncActorRunner, SyncActorRunnerBuilder};

/// Attempts to extract a message from a panic, defaulting to `<unknown>`.
/// NOTE: be sure to derefence the `Box`!
#[doc(hidden)] // Not part of the stable API.
pub fn panic_message<'a>(panic: &'a (dyn std::any::Any + Send + 'static)) -> &'a str {
    match panic.downcast_ref::<&'static str>() {
        Some(s) => s,
        None => match panic.downcast_ref::<String>() {
            Some(s) => s,
            None => "<unknown>",
        },
    }
}
