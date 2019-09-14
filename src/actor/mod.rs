//! The module with the `NewActor` and `Actor` trait definitions.
//!
//! All actors must implement the [`Actor`] trait, which defines how an actor is
//! run. The [`NewActor`] defines how an actor is created. The easiest way to
//! implement these traits is to use asynchronous functions, see the example
//! below.
//!
//! [`Actor`]: crate::actor::Actor
//! [`NewActor`]: crate::actor::NewActor
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
//! // Unfortunately `actor` doesn't yet implement `NewActor`, it first needs to
//! // be cast into a function pointer, which does implement `NewActor`.
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

mod context;

#[cfg(test)]
mod tests;

pub mod messages;
pub mod sync;

pub mod message_select {
    //! Module containing the `MessageSelector` trait and related types.

    #[doc(inline)]
    pub use crate::inbox::{First, MessageSelection, MessageSelector, Messages, Priority};
}

#[doc(inline)]
pub use context::{Context, PeekMessage, ReceiveMessage};

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
    /// use heph::system::RuntimeError;
    /// use heph::{actor, ActorOptions, ActorSystem};
    ///
    /// fn main() -> Result<(), RuntimeError> {
    ///     // Create and run the actor system.
    ///     ActorSystem::new()
    ///         .with_setup(|mut system_ref| {
    ///             // Add the actor to the system.
    ///             let new_actor = actor as fn(_) -> _;
    ///             let mut actor_ref = system_ref.spawn(NoSupervisor, new_actor, (), ActorOptions::default());
    ///
    ///             // Now we can use the reference to send the actor a message.
    ///             // We don't have to use `Message` type we can just use
    ///             // `String`, because `Message` implements `From<String>`.
    ///             actor_ref <<= "Hello world".to_owned();
    ///             Ok(())
    ///         })
    ///         .run()
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
    ///     let msg = ctx.receive_next().await;
    /// #   assert_eq!(msg, Message::String("Hello world".to_owned()));
    ///     println!("received message: {:?}", msg);
    ///     Ok(())
    /// }
    /// ```
    type Message;

    /// The argument(s) passed to the actor.
    ///
    /// The arguments passed to the actor are much like arguments passed to a
    /// regular function. If more then one argument is needed the arguments can
    /// be in the form of a tuple, e.g. `(123, "Hello")`. For example
    /// [`TcpListener`] requires a `NewActor` where the argument  is a tuple
    /// `(TcpStream, SocketAddr)`.
    ///
    /// An empty tuple can be used for actors that don't accept any arguments
    /// (except for the `actor::Context`, see [`new`] below).
    ///
    /// When using asynchronous functions arguments are passed regularly, i.e.
    /// not in the form of a tuple, however they do have be passed as a tuple to
    /// the [`try_spawn`] method. See there [implementations] below.
    ///
    /// [`TcpListener`]: crate::net::TcpListener
    /// [`new`]: NewActor::new
    /// [`try_spawn`]: crate::system::ActorSystemRef::try_spawn
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

    /// Create a new [`Actor`](Actor).
    fn new(
        &mut self,
        ctx: Context<Self::Message>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error>;

    /// Wrap the `NewActor` to change the arguments its accepts.
    ///
    /// This can be used when additional arguments are needed to be passed to an
    /// actor, where another function requires a certain argument list. For
    /// example when using [`tcp::Server`].
    ///
    /// [`tcp::Server`]: crate::net::tcp::Server
    ///
    /// # Examples
    ///
    /// Using [`tcp::Server`] requires a `NewActor` that accepts `(TcpStream,
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
    /// use heph::actor::{self, NewActor};
    /// # use heph::log::error;
    /// use heph::net::tcp::{self, TcpStream};
    /// # use heph::supervisor::{Supervisor, SupervisorStrategy};
    /// use heph::system::{ActorOptions, ActorSystem, ActorSystemRef, RuntimeError};
    ///
    /// fn main() -> Result<(), RuntimeError<io::Error>> {
    ///     // Create and run the actor system.
    ///     ActorSystem::new().with_setup(setup).run()
    /// }
    ///
    /// /// In this setup function we'll add the TcpListener to the actor system.
    /// fn setup(mut system_ref: ActorSystemRef) -> io::Result<()> {
    ///     // Our extra argument.
    ///     let extra: usize = 1;
    ///
    ///     // Our actor that accepts three arguments.
    ///     let new_actor = (conn_actor as fn(_, _, _, _) -> _)
    ///         .map_arg(move |(stream, address)| (stream, address, extra));
    ///
    ///     // For more information about the remainder of this example see
    ///     // `tcp::Server`.
    ///     let address = "127.0.0.1:7890".parse().unwrap();
    ///     let server = tcp::Server::setup(address, conn_supervisor, new_actor, ActorOptions::default())?;
    ///     # let mut actor_ref =
    ///     system_ref.try_spawn(ServerSupervisor, server, (), ActorOptions::default())?;
    ///     # actor_ref <<= Terminate;
    ///     Ok(())
    /// }
    ///
    /// # #[derive(Copy, Clone, Debug)]
    /// # struct ServerSupervisor;
    /// #
    /// # impl<S, NA> Supervisor<tcp::ServerSetup<S, NA>> for ServerSupervisor
    /// # where
    /// #     S: Supervisor<NA> + Clone + 'static,
    /// #     NA: NewActor<Argument = (TcpStream, SocketAddr), Error = !> + Clone + 'static,
    /// # {
    /// #     fn decide(&mut self, err: tcp::ServerError<!>) -> SupervisorStrategy<()> {
    /// #         use tcp::ServerError::*;
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
    /// async fn conn_actor(_ctx: actor::Context<!>, mut stream: TcpStream, address: SocketAddr, extra: usize) -> io::Result<()> {
    /// #   drop(address); // Silence dead code warnings.
    /// #   drop(extra);
    ///     stream.write_all(b"Hello World").await
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
    fn new(
        &mut self,
        ctx: Context<Self::Message>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        let arg = (self.map)(arg);
        self.new_actor.new(ctx, arg)
    }
}

impl<M, A> NewActor for fn(ctx: Context<M>) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = ();
    type Actor = A;
    type Error = !;
    fn new(
        &mut self,
        ctx: Context<Self::Message>,
        _arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx))
    }
}

impl<M, Arg, A> NewActor for fn(ctx: Context<M>, arg: Arg) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = Arg;
    type Actor = A;
    type Error = !;
    fn new(
        &mut self,
        ctx: Context<Self::Message>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg))
    }
}

impl<M, Arg1, Arg2, A> NewActor for fn(ctx: Context<M>, arg1: Arg1, arg2: Arg2) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2);
    type Actor = A;
    type Error = !;
    fn new(
        &mut self,
        ctx: Context<Self::Message>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg.0, arg.1))
    }
}

impl<M, Arg1, Arg2, Arg3, A> NewActor
    for fn(ctx: Context<M>, arg1: Arg1, arg2: Arg2, arg3: Arg3) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3);
    type Actor = A;
    type Error = !;
    fn new(
        &mut self,
        ctx: Context<Self::Message>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg.0, arg.1, arg.2))
    }
}

impl<M, Arg1, Arg2, Arg3, Arg4, A> NewActor
    for fn(ctx: Context<M>, arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3, Arg4);
    type Actor = A;
    type Error = !;
    fn new(
        &mut self,
        ctx: Context<Self::Message>,
        arg: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok((self)(ctx, arg.0, arg.1, arg.2, arg.3))
    }
}

impl<M, Arg1, Arg2, Arg3, Arg4, Arg5, A> NewActor
    for fn(ctx: Context<M>, arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4, arg5: Arg5) -> A
where
    A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3, Arg4, Arg5);
    type Actor = A;
    type Error = !;
    fn new(
        &mut self,
        ctx: Context<Self::Message>,
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
pub trait Bound {
    /// Error type used in [`bind_to`].
    ///
    /// [`bind_to`]: Bound::bind_to
    type Error;

    /// Bind a type to the [`Actor`] that owns the `ctx`.
    fn bind_to<M>(&mut self, ctx: &mut Context<M>) -> Result<(), Self::Error>;
}
