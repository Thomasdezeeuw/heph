//! # Quick Start Guide
//!
//! The document describes some of the core concepts of Heph as quick (10-20
//! minute) introduction to Heph.
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
//! # use heph::rt::ThreadLocal;
//! // The `ThreadLocal` means we're running a thread-local actor, see the
//! // `actor` module for more information about the different kinds of actors.
//! async fn actor(mut ctx: actor::Context<String, ThreadLocal>) {
//!     // Message can be received from the `actor::Context`. In the example we
//!     // can receive messages of the type `String`.
//!     if let Ok(msg) = ctx.receive_next().await {
//!         // Process the message.
//!         println!("got a message: {}", msg);
//!     }
//! }
//! # drop(actor); // Silence dead code warnings.
//! ```
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
//! [`Spawn`] trait, which is implemented on most runtime types, such as
//! [`Runtime`] and [`RuntimeRef`], but we'll get back to those types in the
//! next section.
//!
//! To spawn an actor we need four things:
//! * A [`Supervisor`]. Each actor needs supervisor to determine what to do if
//!   the actor hits an error. For more information see the [supervisor] module.
//! * The [starting argument(s)] for the actor, see the example below.
//! * The [`ActorOptions`]. For now we'll ignore these, they are for more
//!   advanced use cases, out of scope for this quick start guide.
//! * Finally we need the actor to start, or more specifially the [`NewActor`]
//!   implementation. For now we're going to ignore that detail and use an
//!   asynchronous function as actor which implements the `NewActor` trait for
//!   us.
//!
//! Let's take a look at an example that spawn an actor.
//!
//! ```
//! # use std::io::{self, stdout, Write};
//! # use heph::actor;
//! # use heph::actor_ref::ActorRef;
//! # use heph::rt::{RuntimeRef, ThreadLocal};
//! # use heph::spawn::options::ActorOptions;
//! # use heph::supervisor::SupervisorStrategy;
//! // Later on we'll see where we can get a `RuntimeRef`.
//! fn spawn_actor(mut runtime_ref: RuntimeRef) {
//!     // Our supervisor that log errors and stops the actor.
//!     let supervisor = supervisor;
//!     // An unfortunate implementation detail requires us to convert our actor
//!     // into a *function pointer* to implement the required traits.
//!     let actor = actor as fn(_, _, _) -> _;
//!     // The arguments passed to the actor, see the `actor` implementation below.
//!     // Since we want to pass multiple arguments we must do so in the tuple
//!     // notation.
//!     let arguments = ("Hello", true);
//!     // For now we'll use the default options.
//!     let options = ActorOptions::default();
//!     let actor_ref: ActorRef<String> = runtime_ref.spawn_local(supervisor, actor, arguments, options);
//!     # drop(actor_ref); // Silence unused variable warning.
//!     // Now we can use `actor_ref` to send the actor messages, see the
//!     // previous example.
//! }
//!
//! /// Supervisor for [`actor`].
//! fn supervisor(err: io::Error) -> SupervisorStrategy<(&'static str, bool)> {
//!     eprintln!("Actor hit an error: {}", err);
//!     // We'll stop the actor. We could also decide to restart it if we wanted to.
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
//!             write!(stdout(), "{} {} to mars!", greeting, name)?;
//!         } else {
//!             write!(stdout(), "{} {} to earth!", greeting, name)?;
//!         }
//!     }
//!     Ok(())
//! }
//! # drop(spawn_actor); // Silence dead code warnings.
//! ```
//!
//! See the [`Spawn`] trait for more information about spawning actors and see
//! the [supervisor] module for more information about actor supervision, e.g.
//! when to stop and when to restart an actor.
//!
//! [`Spawn`]: crate::spawn::Spawn
//! [`Runtime`]: crate::rt::Runtime
//! [`RuntimeRef`]: crate::rt::RuntimeRef
//! [`Supervisor`]: crate::supervisor::Supervisor
//! [supervisor]: crate::supervisor
//! [starting argument(s)]: crate::actor::NewActor::Argument
//! [`ActorOptions`]: crate::spawn::options::ActorOptions
//! [`NewActor`]: crate::actor::NewActor
//!
//! ## The Heph runtime
//!
//! To run all the actors we need a runtime. Luckily Heph provides one! A
//! runtime can be created by first creating a runtime setup ([`rt::Setup`]),
//! building the runtime ([`Runtime`]) from it and finally starting the runtime.
//! The following example show how to create and run a runtime.
//!
//! ```
//! # #![feature(never_type)]
//! # use heph::rt::{self, Runtime, RuntimeRef};
//! fn main() -> Result<(), rt::Error> {
//!     // First we setup the runtime.
//!     let mut runtime = Runtime::setup()
//!         // For this example we'll use two worker threads. Meaning we'll use
//!         // two threads to run all actors.
//!         .num_threads(2)
//!         // And we build our configured runtime
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
//! [runtime] module. Also take a look at some of the options available on the
//! [`rt::Setup`].
//!
//! Now you know the core concepts of Heph!
//!
//! If you want to look at some more example take a look at the [examples] in
//! the examples directory of the source code.
//!
//! [`rt::Setup`]: crate::rt::Setup
//! [runtime]: crate::rt
//! [examples]: https://github.com/Thomasdezeeuw/heph/blob/master/examples/README.md
