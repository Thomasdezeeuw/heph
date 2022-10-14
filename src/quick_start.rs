//! # Quick Start Guide
//!
//! This document describes some of the core concepts of Heph as quick (10-20
//! minute) introduction to Heph.
//!
//! We'll be using the Heph runtime (the [`heph-rt`] crate), but any [`Future`]
//! runtime can be used.
//!
//! [`heph-rt`]: https://crates.io/crates/heph-rt
//! [`Future`]: std::future::Future
//!
//! ## Actors
//!
//! The most important concept of Heph is an actor. The "actor" terminology
//! comes from the [actor model], in which an actor is an entity that can do
//! three things:
//!  * Receive and process messages.
//!  * Send messages to other actors.
//!  * Spawn new actors.
//!
//! In Heph actors can come in one of three different kinds, however for now
//! we'll use the simplest kind: thread-local actors. To learn more about actors
//! in Heph see the [actor] module. The simplest way to implement an actor is
//! using an asynchronous function, which looks like the following.
//!
//! ```
//! # use heph::actor;
//! # use heph_rt::ThreadLocal;
//! // The `ThreadLocal` means we're running a thread-local actor, see the
//! // `actor` module for more information about the different kinds of actors.
//! async fn actor(mut ctx: actor::Context<String, ThreadLocal>) {
//!     // Messages can be received from the `actor::Context`. In this example we
//!     // can receive messages of the type `String`.
//!     if let Ok(msg) = ctx.receive_next().await {
//!         // Process the message.
//!         println!("got a message: {msg}");
//!     }
//! }
//! # drop(actor); // Silence dead code warnings.
//! ```
//!
//! The example above also shows how an actor can receive and process messages.
//!
//! [actor model]: https://en.wikipedia.org/wiki/Actor_model
//! [actor]: crate::actor
//!
//! ### Sending messages
//!
//! Sending messages is done using the [`ActorRef`] type. An actor reference
//! (`ActorRef`) is a reference to an actor and can be used to send it messages,
//! make a Remote Procedure Call (RPC) or check if it's still alive. The example
//! below shows how to send a message to an actor.
//!
//! ```
//! # use heph::actor_ref::ActorRef;
//! // Later on we'll see how we can get an `ActorRef`.
//! async fn send_message(actor_ref: ActorRef<String>) {
//!     // Send a message to the actor referenced by `ActorRef`.
//!     let msg = "Hello world!".to_owned();
//!     # let _ = // Silence unused `Result` warning.
//!     actor_ref.send(msg).await;
//! }
//! # drop(send_message); // Silence dead code warnings.
//! ```
//!
//! See the [actor reference] module for more information about what actor
//! references can do, including sending messages and RPC.
//!
//! [`ActorRef`]: crate::actor_ref::ActorRef
//! [actor reference]: crate::actor_ref
//!
//! ### Spawning Actors
//!
//! To run an actor it must be spawned. How to spawn an actor is defined by the
//! `Spawn` trait, which is implemented on most runtime types, such as
//! `Runtime` and `RuntimeRef`, but we'll get to those types in the next
//! section.
//!
//! To spawn an actor we need four things:
//! * A [`Supervisor`]. Each actor needs supervision to determine what to do if
//!   the actor hits an error. For more information see the [supervisor] module.
//! * The actor to start, or more specifically the [`NewActor`] implementation.
//!   For now we're going to ignore that detail and use an asynchronous function
//!   as actor which implements the `NewActor` trait for us.
//! * The [starting argument(s)] for the actor, see the example below.
//! * Finally we need `ActorOptions`. For now we'll ignore these as they are for
//!   more advanced use cases, out of scope for this quick start guide.
//!
//! Let's take a look at an example that spawns an actor.
//!
//! ```
//! # use std::io::{self, stdout, Write};
//! # use heph::actor;
//! # use heph::actor_ref::ActorRef;
//! # use heph::supervisor::SupervisorStrategy;
//! # use heph_rt::spawn::options::ActorOptions;
//! # use heph_rt::{RuntimeRef, ThreadLocal};
//! # use log::warn;
//! // Later on we'll see where we can get a `RuntimeRef`.
//! fn spawn_actor(mut runtime_ref: RuntimeRef) {
//!     // Our supervisor for the error.
//!     let supervisor = supervisor;
//!     // An unfortunate implementation detail requires us to convert our actor
//!     // into a *function pointer* to implement the required traits.
//!     let actor = actor as fn(_, _, _) -> _;
//!     // The arguments passed to the actor, see the `actor` implementation below.
//!     // Since we want to pass multiple arguments we must do so in the tuple
//!     // notation.
//!     let arguments = ("Hello", true);
//!     // For this example we'll use the default options.
//!     let options = ActorOptions::default();
//!     let actor_ref: ActorRef<String> = runtime_ref.spawn_local(supervisor, actor, arguments, options);
//!
//!     // Now we can use `actor_ref` to send the actor messages, as shown by the
//!     // previous example.
//!     # drop(actor_ref); // Silence unused variable warning.
//! }
//!
//! /// Supervisor for [`actor`].
//! fn supervisor(err: io::Error) -> SupervisorStrategy<(&'static str, bool)> {
//!     // First we need to handle the error, in this case we'll log it (always a
//!     // good idea).
//!     warn!("Actor hit an error: {err}");
//!
//!     // Then we need to decide if we want to stop or restart the actor. For this
//!     // example we'll stop the actor.
//!     SupervisorStrategy::Stop
//! }
//!
//! /// Our actor with two starting arguments: `greeting` and `on_mars`.
//! async fn actor(
//!     mut ctx: actor::Context<String, ThreadLocal>,
//!     // Our starting arguments, passed as tuple (i.e. `(greeting, on_mars)`) to
//!     // the spawn method.
//!     greeting: &'static str,
//!     on_mars: bool
//! ) -> io::Result<()> {
//!     while let Ok(name) = ctx.receive_next().await {
//!         if on_mars {
//!             write!(stdout(), "{greeting} {name} to mars!")?;
//!         } else {
//!             write!(stdout(), "{greeting} {name} to earth!")?;
//!         }
//!     }
//!     Ok(())
//! }
//! # drop(spawn_actor); // Silence dead code warnings.
//! ```
//!
//! See the `Spawn` trait for more information about spawning actors and see
//! the [supervisor] module for more information about actor supervision, e.g.
//! when to stop and when to restart an actor.
//!
//! [`Supervisor`]: crate::supervisor::Supervisor
//! [supervisor]: crate::supervisor
//! [starting argument(s)]: crate::actor::NewActor::Argument
//! [`NewActor`]: crate::actor::NewActor
//!
//! ## The Heph runtime
//!
//! To run all the actors we need a runtime. Luckily Heph provides one in the
//! `heph-rt` crate! A runtime can be created by first creating a runtime setup
//! (`heph_rt::Setup`), building the runtime (`Runtime`) from it and finally
//! starting the runtime. The following example shows how to do all of the
//! above.
//!
//! ```
//! # #![feature(never_type)]
//! #
//! # use heph_rt::{self as rt, Runtime, RuntimeRef};
//! fn main() -> Result<(), rt::Error> {
//!     // First we setup the runtime and configure it.
//!     let mut runtime = Runtime::setup()
//!         // For this example we'll use two worker threads. Meaning we'll use
//!         // two threads to run all actors.
//!         .num_threads(2)
//!         // And we build our configured runtime.
//!         .build()?;
//!
//!     // Run `spawn_actor` on each worker thread (both of them) to spawn our
//!     // thread-local actor.
//!     runtime.run_on_workers(spawn_actor)?;
//!
//!     // Start the runtime.
//!     // This will wait until all actors are finished running.
//!     runtime.start()
//! }
//!
//! // This is the same function as the one in the previous example.
//! fn spawn_actor(runtime_ref: RuntimeRef) -> Result<(), !> {
//!     // ...
//!     # drop(runtime_ref); // Silence unused variable warning.
//!     # Ok(())
//! }
//! ```
//!
//! For more information about setting up and using the runtime see the
//! `heph_rt`. Also take a look at some of the options available on the
//! `heph_rt::Setup` type.
//!
//! Now you know the core concepts of Heph!
//!
//! If you want to look at some more example take a look at the [examples] in
//! the examples directory of the source code.
//!
//! [examples]: https://github.com/Thomasdezeeuw/heph/blob/master/examples/README.md
