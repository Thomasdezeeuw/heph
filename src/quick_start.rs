//! # Quick Start Guide
//!
//! This document describes some of the core concepts of Heph as a quick (10-20
//! minute) introduction to Heph.
//!
//! We'll be using the Heph runtime (the [Heph-rt] crate), but any [`Future`]
//! runtime can be used.
//!
//! [Heph-rt]: https://crates.io/crates/heph-rt
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
//! In Heph actors can come in one of two different flavours; asynchronous or
//! synchronous actors. Asynchronous actors share a thread with other actors,
//! while a synchronous actor has a thread for itself. To learn more about
//! actors in Heph see the [actor] module.
//!
//! The simplest way to implement an asynchronous actor is using an
//! `async`hronous function, which looks like the following.
//!
//! ```
//! use heph::actor;
//!
//! async fn actor(mut ctx: actor::Context<String>) {
//!     // Messages can be received from the `actor::Context`. In this example we
//!     // can receive messages of the type `String`.
//!     while let Ok(msg) = ctx.receive_next().await {
//!         // Process the message.
//!         println!("got a message: {msg}");
//!     }
//! }
//! # _ = actor; // Silence dead code warnings.
//! ```
//!
//! The example above also shows how an actor can receive and process messages.
//! This simple function already shows the first point of the actor model
//! (receiving and processing message).
//!
//! [actor model]: https://en.wikipedia.org/wiki/Actor_model
//! [actor]: crate::actor
//!
//! ### Sending Messages
//!
//! On to the second point of the actor model: sending message. Sending messages
//! is done using the [`ActorRef`] type. An actor reference (`ActorRef`) is a
//! reference to an actor and can be used to send it messages, make a Remote
//! Procedure Call (RPC) or check if it's still alive.
//!
//! The example below shows an actor that acts as a filter, it only allows
//! messages that end with "please". Later on we'll see how we can get an
//! `ActorRef`.
//!
//! ```
//! use heph::{actor, ActorRef};
//! use heph::actor_ref::SendError;
//!
//! async fn filter(mut ctx: actor::Context<String>, actor_ref: ActorRef<String>) -> Result<(), SendError> {
//!     // Same receive loop we've seen before.
//!     while let Ok(msg) = ctx.receive_next().await {
//!         if msg.ends_with("please") {
//!             // We send nice messages along to the actor we reference with
//!             // `actor_ref`.
//!             actor_ref.send(msg).await?;
//!         } else {
//!             println!("Someone wasn't taught any manners!");
//!         }
//!     }
//!     Ok(())
//! }
//! # _ = filter; // Silence dead code warnings.
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
//! Now on to the third and final point of the actor model: spawning actors. To
//! run an actor it must be spawned, or in other words added to the runtime and
//! ran (but that's quite long so we often use "spawn" instead).
//!
//! How to spawn an actor is different for the runtime used. In the example
//! below we'll use the [Heph-rt] runtime, but it's possible to spawn actors as
//! [`Future`]s, for that see [`ActorFuture`].
//!
//! For Heph-rt the spawning of actors is defined by the [`Spawn`] trait, which
//! is implemented on most runtime types, such as `Runtime` and `RuntimeRef`,
//! we'll see how get to those types in the next section.
//!
//! To spawn an actor we need four things (this is true for both the `Spawn`
//! trait and `ActorFuture`):
//! * A [`Supervisor`]. Each actor needs supervision to determine what to do if
//!   the actor hits an error. For more information see the [supervisor] module.
//! * The actor to start, or more specifically the [`NewActor`] implementation.
//!   For now we're going to ignore that detail and use an asynchronous function
//!   as actor which implements the `NewActor` trait for us.
//! * The [starting argument(s)] for the actor, see the example below.
//! * For Heph-rt specifically we need `ActorOptions`, these options control how
//!   the actor is run within the runtime. For now we'll ignore these as they are
//!   for more advanced use cases, out of scope for this quick start guide.
//!
//! Let's take a look at an example that spawns an actor for each incoming
//! message.
//!
//! ```
//! use std::io::{self, stdout, Write};
//! use heph::actor::{self, actor_fn};
//! use heph::actor_ref::{ActorRef, SendError};
//! use heph::supervisor::SupervisorStrategy;
//! use heph_rt::spawn::options::ActorOptions;
//! use heph_rt::ThreadLocal;
//!
//! async fn spawn_actor(mut ctx: actor::Context<(String, bool), ThreadLocal>) -> Result<(), SendError> {
//!     // The same receive loop we've twice before.
//!     while let Ok((msg, on_mars)) = ctx.receive_next().await {
//!         // Our supervisor for the error, see the function below.
//!         let supervisor = greeter_supervisor;
//!         // An unfortunate implementation detail requires us to convert our
//!         // asynchronous function to an `NewActor` implementation (trait
//!         // that defines how actors are (re)started).
//!         // The easiest way to do this is to use the `actor_fn` helper
//!         // function. See the documentation of `actor_fn` for more details on
//!         // why this is required.
//!         let actor = actor_fn(greeter_actor);
//!         // The arguments passed to the actor, see the `actor` implementation
//!         // below. Since we want to pass multiple arguments we must do so
//!         // using the tuple notation.
//!         let arguments = (msg, on_mars);
//!         // For this example we'll use the default options.
//!         let options = ActorOptions::default();
//!
//!         // With all arguments prepared we can do the actual spawning of the
//!         // actor.
//!         let actor_ref: ActorRef<String> = ctx.runtime().spawn_local(supervisor, actor, arguments, options);
//!
//!         // In return we get an `ActorRef`, which we just learned can be used
//!         // to send the actor messages.
//!         actor_ref.send(String::from("Alice")).await?;
//!     }
//!     Ok(())
//! }
//!
//! /// Supervisor for [`greeter_actor`].
//! fn greeter_supervisor(err: io::Error) -> SupervisorStrategy<(String, bool)> {
//!     // First we need to handle the error, in this case we'll log it (always
//!     // a good idea).
//!     log::warn!("greeter actor hit an error: {err}");
//!
//!     // Then we need to decide if we want to stop or restart the actor. For
//!     // this example we'll stop the actor.
//!     SupervisorStrategy::Stop
//! }
//!
//! /// Our actor with two starting arguments: `greeting` and `on_mars`.
//! async fn greeter_actor(
//!     mut ctx: actor::Context<String, ThreadLocal>,
//!     // Our starting arguments, passed as tuple to the spawn method.
//!     greeting: String,
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
//! # _ = spawn_actor; // Silence dead code warnings.
//! ```
//!
//! See the [`Spawn`] trait for more information about spawning actors and see
//! the [supervisor] module for more information about actor supervision, e.g.
//! when to stop and when to restart an actor.
//!
//! [`Future`]: std::future::Future
//! [`ActorFuture`]: crate::ActorFuture
//! [`Spawn`]: https://docs.rs/heph-rt/latest/heph_rt/spawn/trait.Spawn.html
//! [`Supervisor`]: crate::supervisor::Supervisor
//! [supervisor]: crate::supervisor
//! [starting argument(s)]: crate::actor::NewActor::Argument
//! [`NewActor`]: crate::actor::NewActor
//!
//! ## The Heph Runtime
//!
//! To run all the actors we need a runtime. Luckily Heph provides one in the
//! [Heph-rt] crate!
//!
//! A Heph runtime can be created by first creating a runtime setup
//! ([`heph_rt::Setup`]), building the runtime ([`Runtime`]) from it and finally
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
//!     // thread-local actors.
//!     runtime.run_on_workers(spawn_actor)?;
//!
//!     // Start the runtime.
//!     // This will wait until all actors are finished running.
//!     runtime.start()
//! }
//!
//! fn spawn_actor(runtime_ref: RuntimeRef) -> Result<(), !> {
//!     // Here we can spawn spawn actor as shown above, for example:
//!     // runtime_ref.spawn_local(supervisor, actor, arguments, options);
//!     # drop(runtime_ref); // Silence unused variable warning.
//!     Ok(())
//! }
//! ```
//!
//! For more information about setting up and using the runtime see the
//! [Heph-rt]. Also take a look at some of the options available on the
//! `heph_rt::Setup` type.
//!
//! Now you know the core concepts of Heph!
//!
//! If you want to look at some more example take a look at the [examples] in
//! the examples directory of the source code.
//!
//! [examples]: https://github.com/Thomasdezeeuw/heph/blob/main/examples/README.md
//! [`heph_rt::Setup`]: https://docs.rs/heph-rt/latest/heph_rt/struct.Setup.html
//! [`Runtime`]: https://docs.rs/heph-rt/latest/heph_rt/struct.Runtime.html
//!
//! ## Not Using the Heph Runtime
//!
//! If you already have a [`Future`] runtime you're happy with you can still use
//! Heph. Instead of using `heph-rt` specific ways to spawn an actor you can use
//! the [`ActorFuture`] type and spawn that on your existing runtime. The
//! example below shows how to do just that.
//!
//! ```
//! # #![feature(never_type)]
//! use heph::ActorFuture;
//! use heph::actor::{self, actor_fn};
//! use heph::supervisor::NoSupervisor;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // The `Supervisor`, `NewActor` and argument are the same as for using
//!     // heph-rt.
//!     let supervisor = NoSupervisor;
//!     let new_actor = actor_fn(actor);
//!     let arguments = (); // No arguments required.
//!     let (actor, actor_ref) = ActorFuture::new(supervisor, new_actor, arguments)?;
//!
//!     // Actor reference can be used as normal.
//!     actor_ref.try_send("Hello!")?;
//!
//!     // Spawn the `Future` that runs the actor using your runtime.
//! #   let my_runtime = RT;
//!     my_runtime.spawn(actor);
//!     Ok(())
//! }
//! #
//! # struct RT;
//! # impl RT { fn spawn<Fut: std::future::Future>(&self, fut: Fut) { drop(fut) } }
//!
//! async fn actor(mut ctx: actor::Context<String>) {
//!     // Messages can be received from the `actor::Context`. In this example we
//!     // can receive messages of the type `String`.
//!     while let Ok(msg) = ctx.receive_next().await {
//!         // Process the message.
//!         println!("got a message: {msg}");
//!     }
//! }
//! ```
//!
//! [`ActorFuture`]: crate::actor::ActorFuture
