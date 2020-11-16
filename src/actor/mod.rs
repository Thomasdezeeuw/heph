//! The module with the `Actor` trait and related definitions.
//!
//! Actors come in three different kinds:
//!
//! * Asynchronous thread-local actors,
//! * Asynchronous thread-safe actors, and
//! * Synchronous actors.
//!
//! Both asynchronous actors must implement the [`Actor`] trait, which defines
//! how an actor is run. The [`NewActor`] defines how an actor is created and is
//! used in staring, or spawning, new actors. The easiest way to implement these
//! traits is to use asynchronous functions, see the example below.
//!
//! The sections below describe each, including up- and downsides of each kind.
//!
//! [`Actor`]: crate::actor::Actor
//! [`NewActor`]: crate::actor::NewActor
//!
//! # Asynchronous thread-local actors
//!
//! Asynchronous thread-local actors, often referred to as just thread-local
//! actors, are actors that will remain on the thread on which they are started.
//! They can be started, or spawned, using [`RuntimeRef::try_spawn_local`], or
//! any type that implements the [`Spawn`] trait using the [`ThreadLocal`]
//! context. These should be the most used as they are the cheapest to run.
//!
//! The upside of running a thread-local actor is that it doesn't have to be
//! [`Send`] or [`Sync`], allowing it to use cheaper types that don't require
//! synchronisation. The downside is that if a single actor blocks it will block
//! *all* actors on the thread. Something that some frameworks work around with
//! actor/tasks that transparently move between threads and hide blocking/bad
//! actors, Heph does not however (for thread-local actor).
//!
//! [`RuntimeRef::try_spawn_local`]: crate::rt::RuntimeRef::try_spawn_local
//! [`ThreadLocal`]: context::ThreadLocal
//!
//! # Asynchronous thread-safe actors
//!
//! Asynchronous thread-safe actors, or just thread-safe actor, are actors that
//! can be run on any of the worker threads and transparently move between them.
//! They can be spawned using [`RuntimeRef::try_spawn`], or any type that
//! implements the [`Spawn`] trait using the [`ThreadSafe`] context. Because
//! these actor move between threads they are required to be [`Send`] and
//! [`Sync`].
//!
//! An upside to using thread-safe actors is that a bad actor (that blocks) only
//! blocks a single worker thread at a time, allowing the other worker threads
//! to run the other thread-safe actors (but not the thread-local actors!). A
//! downside is that these actors are more expansive to run than thread-local
//! actors.
//!
//! [`RuntimeRef::try_spawn`]: crate::rt::RuntimeRef::try_spawn
//! [`ThreadSafe`]: context::ThreadSafe
//!
//! # Synchronous actors
//!
//! The previous two actors, thread-local and thread-safe actors, are not
//! allowed to block the thread they run on, as that would block all other
//! actors on that thread. However sometimes blocking operations is exactly what
//! we need to do, for that purpose Heph has synchronous actors.
//!
//! Synchronous actors run own there own thread and can use blocking operations,
//! such as blocking I/O. The [`sync`] module and the [`SyncActor`] trait have
//! more information about synchronous actors.
//!
//! [`SyncActor`]: crate::actor::sync::SyncActor
//!
//! # Example
//!
//! Using an asynchronous function to implement the `NewActor` and `Actor`
//! traits.
//!
//! ```
//! use heph::{actor, NewActor};
//!
//! async fn actor(ctx: actor::Context<()>) -> Result<(), ()> {
//! #   drop(ctx); // Use `ctx` to silence dead code warnings.
//!     println!("Actor is running!");
//!     Ok(())
//! }
//!
//! // Unfortunately `actor` doesn't yet implement `NewActor`, it first needs
//! // to be cast into a function pointer, which does implement `NewActor`.
//! use_actor(actor as fn(_) -> _);
//!
//! fn use_actor<NA>(new_actor: NA) where NA: NewActor {
//!     // Do stuff with the actor ...
//! #   drop(new_actor);
//! }
//! ```

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{self, Poll};

use crate::rt;

#[path = "context.rs"]
mod context_priv;
mod spawn;

pub mod messages;
pub mod sync;

pub mod context {
    //! Module containing the different kinds of contexts.
    //!
    //! See [`actor::Context`] for more information.
    //!
    //! [`actor::Context`]: crate::actor::Context

    #[doc(inline)]
    pub use super::context_priv::{ThreadLocal, ThreadSafe};
}

#[doc(inline)]
pub use context_priv::{Context, NoMessages, ReceiveMessage, RecvError};
#[doc(inline)]
pub use spawn::Spawn;

pub(crate) use spawn::{AddActorError, PrivateSpawn};

/// The trait that defines how to create a new [`Actor`].
///
/// The easiest way to implement this by using an asynchronous function, see the
/// [module level] documentation.
///
/// [module level]: crate::actor
pub trait NewActor {
    /// The type of messages the actor can receive.
    ///
    /// Using an enum allows an actor to handle multiple types of messages.
    ///
    /// # Examples
    ///
    /// Here is an example of using an enum as message type.
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use heph::supervisor::NoSupervisor;
    /// use heph::{actor, rt, ActorOptions, Runtime};
    ///
    /// fn main() -> Result<(), rt::Error> {
    ///     // Create and run the runtime.
    ///     Runtime::new()?
    ///         .with_setup(|mut runtime_ref| {
    ///             // Spawn the actor.
    ///             let new_actor = actor as fn(_) -> _;
    ///             let actor_ref = runtime_ref.spawn_local(NoSupervisor, new_actor, (),
    ///                 ActorOptions::default());
    ///
    ///             // Now we can use the reference to send the actor a message.
    ///             // We don't have to use `Message` type we can just use
    ///             // `String`, because `Message` implements `From<String>`.
    ///             actor_ref.try_send("Hello world".to_owned()).unwrap();
    ///             Ok(())
    ///         })
    ///         .start()
    /// }
    ///
    /// /// The message type for the actor.
    /// #[derive(Debug)]
    /// # #[derive(Eq, PartialEq)]
    /// enum Message {
    ///     String(String),
    /// #   #[allow(dead_code)]
    ///     Number(usize),
    /// }
    ///
    /// // Implementing `From` for the message allows us to just pass a
    /// // `String`, rather then a `Message::String`. See sending of the
    /// // message in the `setup` function.
    /// impl From<String> for Message {
    ///     fn from(str: String) -> Message {
    ///         Message::String(str)
    ///     }
    /// }
    ///
    /// /// Our actor implementation that prints all messages it receives.
    /// async fn actor(mut ctx: actor::Context<Message>) -> Result<(), !> {
    ///     if let Ok(msg) = ctx.receive_next().await {
    /// #       assert_eq!(msg, Message::String("Hello world".to_owned()));
    ///         println!("received message: {:?}", msg);
    ///     }
    ///     Ok(())
    /// }
    /// ```
    type Message;

    /// The argument(s) passed to the actor.
    ///
    /// The arguments passed to the actor are much like arguments passed to a
    /// regular function. If more then one argument is needed the arguments can
    /// be in the form of a tuple, e.g. `(123, "Hello")`. For example
    /// [`TcpServer`] requires a `NewActor` where the argument  is a tuple
    /// `(TcpStream, SocketAddr)`.
    ///
    /// An empty tuple can be used for actors that don't accept any arguments
    /// (except for the `actor::Context`, see [`new`] below).
    ///
    /// When using asynchronous functions arguments are passed regularly, i.e.
    /// not in the form of a tuple, however they do have be passed as a tuple to
    /// the [`try_spawn_local`] method. See there [implementations] below.
    ///
    /// [`TcpServer`]: crate::net::TcpServer
    /// [`new`]: NewActor::new
    /// [`try_spawn_local`]: crate::RuntimeRef::try_spawn_local
    /// [implementations]: #foreign-impls
    type Argument;

    /// The type of the actor.
    ///
    /// See [`Actor`](Actor) for more.
    type Actor: Actor;

    /// The type of error.
    ///
    /// Note that if creating an actor is always successful the never type (`!`)
    /// can be used. Asynchronous functions for example use the never type as
    /// error.
    type Error;

    /// The kind of actor context.
    ///
    /// See [`actor::Context`] for more information.
    ///
    /// [`actor::Context`]: crate::actor::Context
    type Context;

    /// Create a new [`Actor`](Actor).
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::Context>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error>;

    /// Wrap the `NewActor` to change the arguments its accepts.
    ///
    /// This can be used when additional arguments are needed to be passed to an
    /// actor, where another function requires a certain argument list. For
    /// example when using [`TcpServer`].
    ///
    /// [`TcpServer`]: crate::net::TcpServer
    ///
    /// # Examples
    ///
    /// Using [`TcpServer`] requires a `NewActor` that accepts `(TcpStream,
    /// SocketAddr)` as arguments, but we need to pass the actor additional
    /// arguments.
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use std::io;
    /// use std::net::SocketAddr;
    ///
    /// use futures_util::AsyncWriteExt;
    ///
    /// # use heph::actor::messages::Terminate;
    /// # use heph::actor::context;
    /// use heph::actor::{self, NewActor};
    /// # use heph::log::error;
    /// use heph::net::{TcpServer, TcpStream};
    /// # use heph::net::tcp::server;
    /// # use heph::supervisor::{Supervisor, SupervisorStrategy};
    /// use heph::{rt, ActorOptions, Runtime, RuntimeRef};
    ///
    /// fn main() -> Result<(), rt::Error<io::Error>> {
    ///     // Create and run runtime
    ///     Runtime::new().map_err(rt::Error::map_type)?.with_setup(setup).start()
    /// }
    ///
    /// /// In this setup function we'll spawn the `TcpServer` actor.
    /// fn setup(mut runtime_ref: RuntimeRef) -> io::Result<()> {
    ///     // Prepare for humans' expand to Mars.
    ///     let greet_mars = true;
    ///
    ///     // Our actor that accepts three arguments.
    ///     let new_actor = (conn_actor as fn(_, _, _, _) -> _)
    ///         .map_arg(move |(stream, address)| (stream, address, greet_mars));
    ///
    ///     // For more information about the remainder of this example see
    ///     // `TcpServer`.
    ///     let address = "127.0.0.1:7890".parse().unwrap();
    ///     let server = TcpServer::setup(address, conn_supervisor, new_actor,
    ///         ActorOptions::default())?;
    ///     # let actor_ref =
    ///     runtime_ref.try_spawn_local(ServerSupervisor, server, (), ActorOptions::default())?;
    ///     # actor_ref.try_send(Terminate).unwrap();
    ///     Ok(())
    /// }
    ///
    /// # #[derive(Copy, Clone, Debug)]
    /// # struct ServerSupervisor;
    /// #
    /// # impl<S, NA> Supervisor<server::Setup<S, NA>> for ServerSupervisor
    /// # where
    /// #     S: Supervisor<NA> + Clone + 'static,
    /// #     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !, Context = context::ThreadLocal> + Clone + 'static,
    /// # {
    /// #     fn decide(&mut self, err: server::Error<!>) -> SupervisorStrategy<()> {
    /// #         use server::Error::*;
    /// #         match err {
    /// #             Accept(err) => {
    /// #                 error!("error accepting new connection: {}", err);
    /// #                 SupervisorStrategy::Restart(())
    /// #             }
    /// #             NewActor(_) => unreachable!(),
    /// #         }
    /// #     }
    /// #
    /// #     fn decide_on_restart_error(&mut self, err: io::Error) -> SupervisorStrategy<()> {
    /// #         error!("error restarting the TCP server: {}", err);
    /// #         SupervisorStrategy::Stop
    /// #     }
    /// #
    /// #     fn second_restart_error(&mut self, _: io::Error) {
    /// #         // We don't restart a second time, so this will never be called.
    /// #         unreachable!();
    /// #     }
    /// # }
    /// #
    /// # fn conn_supervisor(err: io::Error) -> SupervisorStrategy<(TcpStream, SocketAddr)> {
    /// #   error!("error handling connection: {}", err);
    /// #   SupervisorStrategy::Stop
    /// # }
    /// #
    /// // Actor that handles a connection.
    /// async fn conn_actor(
    ///     _ctx: actor::Context<!>,
    ///     mut stream: TcpStream,
    ///     address: SocketAddr,
    ///     greet_mars: bool
    /// ) -> io::Result<()> {
    /// #   drop(address); // Silence dead code warnings.
    ///     if greet_mars {
    ///         // In case this example ever reaches Mars.
    ///         stream.write_all(b"Hello Mars").await
    ///     } else {
    ///         stream.write_all(b"Hello World").await
    ///     }
    /// }
    /// ```
    fn map_arg<F, Arg>(self, f: F) -> ArgMap<Self, F, Arg>
    where
        Self: Sized,
        F: FnMut(Arg) -> Self::Argument,
    {
        ArgMap {
            new_actor: self,
            map: f,
            _phantom: PhantomData,
        }
    }
}

/// See [`NewActor::map_arg`].
#[derive(Debug)]
pub struct ArgMap<NA, F, Arg> {
    new_actor: NA,
    map: F,
    _phantom: PhantomData<Arg>,
}

impl<NA, F, Arg> Clone for ArgMap<NA, F, Arg>
where
    NA: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        ArgMap {
            new_actor: self.new_actor.clone(),
            map: self.map.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<NA, F, Arg> NewActor for ArgMap<NA, F, Arg>
where
    NA: NewActor,
    F: FnMut(Arg) -> NA::Argument,
{
    type Message = NA::Message;
    type Argument = Arg;
    type Actor = NA::Actor;
    type Error = NA::Error;
    type Context = NA::Context;
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::Context>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let arg = (self.map)(arg);
        self.new_actor.new(ctx, arg)
    }
}

impl<M, C, A> NewActor for fn(ctx: Context<M, C>) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = ();
    type Actor = A;
    type Error = !;
    type Context = C;
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::Context>,
        _arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx))
    }
}

impl<M, C, Arg, A> NewActor for fn(ctx: Context<M, C>, arg: Arg) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = Arg;
    type Actor = A;
    type Error = !;
    type Context = C;
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::Context>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg))
    }
}

impl<M, C, Arg1, Arg2, A> NewActor for fn(ctx: Context<M, C>, arg1: Arg1, arg2: Arg2) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2);
    type Actor = A;
    type Error = !;
    type Context = C;
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::Context>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg.0, arg.1))
    }
}

impl<M, C, Arg1, Arg2, Arg3, A> NewActor
    for fn(ctx: Context<M, C>, arg1: Arg1, arg2: Arg2, arg3: Arg3) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3);
    type Actor = A;
    type Error = !;
    type Context = C;
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::Context>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg.0, arg.1, arg.2))
    }
}

impl<M, C, Arg1, Arg2, Arg3, Arg4, A> NewActor
    for fn(ctx: Context<M, C>, arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3, Arg4);
    type Actor = A;
    type Error = !;
    type Context = C;
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::Context>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg.0, arg.1, arg.2, arg.3))
    }
}

impl<M, C, Arg1, Arg2, Arg3, Arg4, Arg5, A> NewActor
    for fn(ctx: Context<M, C>, arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4, arg5: Arg5) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3, Arg4, Arg5);
    type Actor = A;
    type Error = !;
    type Context = C;
    fn new(
        &mut self,
        ctx: Context<Self::Message, Self::Context>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg.0, arg.1, arg.2, arg.3, arg.4))
    }
}

/// The `Actor` trait defines how the actor is run.
///
/// Effectively an `Actor` is a [`Future`] which returns a `Result<(), Error>`,
/// where `Error` is defined on the trait. That is why there is a blanket
/// implementation for all `Future`s with a `Result<(), Error>` as `Output`
/// type.
///
/// The easiest way to implement this by using an async function, see the
/// [module level] documentation.
///
/// [module level]: crate::actor
///
/// # Panics
///
/// Because this is basically a [`Future`] it also shares it's characteristics,
/// including it's unsafety. Please read the [`Future`] documentation when
/// implementing or using this by hand.
pub trait Actor {
    /// An error the actor can return to its [supervisor]. This error will be
    /// considered terminal for this actor and should **not** be an error of
    /// regular processing of a message.
    ///
    /// How to process non-terminal errors that happen during regular processing
    /// is up to the actor.
    ///
    /// [supervisor]: crate::supervisor
    type Error;

    /// Try to poll this actor.
    ///
    /// This is basically the same as calling [`Future::poll`].
    ///
    /// # Panics
    ///
    /// Just like with [`Future`]s polling after it returned [`Poll::Ready`] may
    /// cause undefined behaviour, including but not limited to panicking.
    fn try_poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>)
        -> Poll<Result<(), Self::Error>>;
}

impl<Fut, E> Actor for Fut
where
    Fut: Future<Output = Result<(), E>>,
{
    type Error = E;

    fn try_poll(
        self: Pin<&mut Self>,
        ctx: &mut task::Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.poll(ctx)
    }
}

/// Types that are bound to an [`Actor`].
///
/// A marker trait to indicate the type is bound to an [`Actor`]. How the type
/// is bound to the actor is different for each type. For most futures it means
/// that if progress can be made (when the [future is awoken]) the actor will be
/// run. This has the unfortunate consequence that those types can't be moved
/// away from the actor without [(re)binding] it first, otherwise the new actor
/// will never be run and the actor that created the type will run instead.
///
/// Most types that are bound can only be created with a (mutable) reference to
/// an [`actor::Context`]. Examples of this are [`TcpStream`], [`UdpSocket`] and
/// all futures in the [`timer`] module.
///
/// [future is awoken]: std::task::Waker::wake
/// [(re)binding]: Bound::bind_to
/// [`actor::Context`]: Context
/// [`TcpStream`]: crate::net::TcpStream
/// [`UdpSocket`]: crate::net::UdpSocket
/// [`timer`]: crate::timer
pub trait Bound<C> {
    /// Error type used in [`bind_to`].
    ///
    /// [`bind_to`]: Bound::bind_to
    type Error;

    /// Bind a type to the [`Actor`] that owns the `ctx`.
    fn bind_to<M>(&mut self, ctx: &mut Context<M, C>) -> Result<(), Self::Error>
    where
        Context<M, C>: rt::Access;
}
