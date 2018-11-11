//! Heph derived from Hephaestus, is the Greek god of blacksmiths, metalworking,
//! carpenters, craftsmen, artisans, sculptors, metallurgy, fire, and volcanoes.
//! <sup>[1]</sup> Well this crate has very little to do with Greek gods, but I
//! needed a name.
//!
//!
//! ## About
//!
//! Heph is an actor <sup>[2]</sup> framework based on asynchronous functions.
//! Such an asynchronous function looks like this:
//!
//! ```rust
//! # #![feature(async_await, await_macro, futures_api, never_type)]
//! #
//! # use heph::actor::ActorContext;
//! #
//! async fn print_actor(mut ctx: ActorContext<String>, _: ()) -> Result<(), !> {
//!     // Receive a message.
//!     let msg = await!(ctx.receive());
//!     // Print the message.
//!     println!("got a message: {}", msg);
//!     // And we're done.
//!     Ok(())
//! }
//! ```
//!
//! Heph uses an event-driven, non-blocking I/O, share nothing design. But what
//! do all those buzzwords actually mean?
//!
//!  - *Event-driven*: Heph does nothing by itself, it must first get an event
//!    before it starts doing anything. For example when using an `TcpListener`
//!    it waits on a notification from the OS saying the `TcpListener` is ready
//!    before trying to accept connections.
//!  - *Non-blocking I/O*: normal I/O operations need to wait (block) until the
//!    operation can complete. Using non-blocking, or asynchronous, I/O means
//!    that rather then waiting for the operation to complete we'll do some
//!    other, more useful, work and try the operation later.
//!  - *Share nothing*: a lot of application share data across multiple threads.
//!    To do this safely we need to protect it from data races, via a [`Mutex`]
//!    or by using [atomic] operations. Heph is designed to not share any data.
//!    Each actor is responsible for its own memory and cannot access memory
//!    owned by other actors. Instead communication is done via sending
//!    messages, see the [actor model].
//!
//! [`Mutex`]: https://doc.rust-lang.org/std/sync/struct.Mutex.html
//! [atomic]: https://doc.rust-lang.org/std/sync/atomic/index.html
//! [actor model]: https://en.wikipedia.org/wiki/Actor_model
//!
//!
//! ## Getting started
//!
//! The easiest way to get start with Heph is looking at the examples in the
//! examples directory of the source code. Or by looking through the API
//! documentation, starting with [`ActorSystem`].
//!
//! [`ActorSystem`]: system/struct.ActorSystem.html
//!
//!
//! ## Features
//!
//! This crate has a single optional feature: `test`. This feature will enable
//! the `test` module which adds testing facilities.
//!
//! [1]: https://en.wikipedia.org/wiki/Hephaestus
//! [2]: https://en.wikipedia.org/wiki/Actor_model

#![feature(arbitrary_self_types,
           async_await,
           await_macro,
           const_fn,
           futures_api,
           never_type,
           non_exhaustive,
           pin,
           read_initializer,
)]

#![warn(anonymous_parameters,
        bare_trait_objects,
        missing_debug_implementations,
        missing_docs,
        trivial_casts,
        trivial_numeric_casts,
        unused_extern_crates,
        unused_import_braces,
        unused_qualifications,
        unused_results,
        variant_size_differences,
)]

pub mod actor;
pub mod actor_ref;
pub mod initiator;
pub mod log;
pub mod net;
pub mod supervisor;
pub mod system;
pub mod timer;

#[cfg(feature = "test")]
pub mod test;

#[cfg(all(test, not(feature = "test")))]
compile_error!("please enable the `test` feature when testing, call `cargo test --features test`");

mod mailbox;
mod process;
mod scheduler;
mod util;
mod waker;

/// The prelude. All useful traits and types in a single module.
///
/// ```
/// use heph::prelude::*;
/// ```
pub mod prelude {
    #[doc(no_inline)]
    pub use crate::actor::{actor_factory, Actor, ActorContext, NewActor};
    #[doc(no_inline)]
    pub use crate::actor_ref::{ActorRef, LocalActorRef};
    #[doc(no_inline)]
    pub use crate::system::{ActorOptions, ActorSystem, ActorSystemRef, InitiatorOptions};
}
