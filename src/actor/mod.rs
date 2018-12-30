//! The module with the `NewActor` and `Actor` trait definitions.
//!
//! All actors must implement the [`Actor`] trait, which defines how an actor is
//! run. The [`NewActor`] defines how an actor is created. The easiest way to
//! implement these traits is to use async functions, see the example below.
//!
//! [`NewActor`]: trait.NewActor.html
//! [`Actor`]: trait.Actor.html
//!
//! # Example
//!
//! Using an asynchronous function to implement the `NewActor` and `Actor`
//! traits.
//!
//! ```
//! #![feature(async_await, futures_api)]
//!
//! use heph::actor::{ActorContext, actor_factory};
//!
//! async fn actor(mut ctx: ActorContext<()>, _arg: ()) -> Result<(), ()> {
//!     println!("Actor is running!");
//!     Ok(())
//! }
//!
//! // Our `NewActor` implementation that returns an `Actor` that runs the
//! // `actor` function.
//! let new_actor = actor_factory(actor);
//! #
//! # fn use_new_actor<NA: heph::actor::NewActor>(new_actor: NA) { }
//! # use_new_actor(new_actor);
//! ```

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{LocalWaker, Poll};

mod context;

#[cfg(all(test, feature = "test"))]
mod tests;

pub use self::context::{ActorContext, ReceiveFuture};

/// The trait that defines how to create a new actor.
///
/// # Examples
///
/// The easiest way to implement this is by using an async function.
///
/// ```
/// #![feature(async_await, await_macro, futures_api)]
///
/// use heph::actor::{actor_factory, ActorContext};
///
/// // Having a async function like the following:
/// async fn greeter_actor(mut ctx: ActorContext<String>, message: String) -> Result<(), ()> {
///     loop {
///         let name = await!(ctx.receive());
///         println!("{} {}", message, name);
///     }
/// }
///
/// // `NewActor` can be implemented using the `actor_factory`.
/// let new_actor = actor_factory(greeter_actor);
/// ```
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
    /// #![feature(async_await, await_macro, futures_api, never_type)]
    ///
    /// use heph::actor::{actor_factory, ActorContext};
    /// use heph::supervisor::NoopSupervisor;
    /// use heph::system::{ActorOptions, ActorSystem};
    ///
    /// // The message type for the actor.
    /// #[derive(Debug)]
    /// enum Message {
    ///     String(String),
    ///     Number(usize),
    /// }
    ///
    /// // Implementing `From` for the message allows us to just pass a
    /// // `String`, rather then a `Message::String`. See sending of the
    /// // message below.
    /// impl From<String> for Message {
    ///     fn from(str: String) -> Message {
    ///         Message::String(str)
    ///     }
    /// }
    ///
    /// // Our actor.
    /// async fn actor(mut ctx: ActorContext<Message>, _arg: ()) -> Result<(), !> {
    ///     loop {
    ///         let msg = await!(ctx.receive());
    ///         println!("received message: {:?}", msg);
    ///     }
    /// }
    ///
    /// // Create and run an `ActorSystem`.
    /// ActorSystem::new()
    ///     .with_setup(|mut system_ref| {
    ///         // Add the actor to the system.
    ///         let new_actor = actor_factory(actor);
    ///         let mut actor_ref = system_ref.spawn(NoopSupervisor, new_actor, (), ActorOptions::default());
    ///
    ///         // Now we can use the reference to send the actor a message, without
    ///         // having to use `Message` we can just use `String`.
    ///         actor_ref.send("Hello world".to_owned());
    ///         Ok(())
    ///     })
    ///     .run()
    ///     .unwrap();
    /// ```
    type Message;

    /// The argument(s) passed to the actor.
    ///
    /// This could for example be a TCP connection the actor is responsible for.
    /// See [`TcpListener`] for an example usage of this. To supply multiple
    /// arguments to an actor a tuple can be used. Some actors don't need
    /// arguments, those actors can use `()` the empty tuple.
    ///
    /// [`TcpListener`]: ../net/struct.TcpListener.html
    type Argument;

    /// The type of the actor.
    ///
    /// See [`Actor`] for more.
    ///
    /// [`Actor`]: trait.Actor.html
    type Actor: Actor;

    /// Create a new `Actor`.
    fn new(&mut self, ctx: ActorContext<Self::Message>, arg: Self::Argument) -> Self::Actor;
}

impl<M, A> NewActor for fn(ActorContext<M>) -> A
    where A: Actor,
{
    type Message = M;
    type Argument = ();
    type Actor = A;
    fn new(&mut self, ctx: ActorContext<Self::Message>, _arg: Self::Argument) -> Self::Actor {
        (self)(ctx)
    }
}

impl<M, Arg, A> NewActor for fn(ActorContext<M>, Arg) -> A
    where A: Actor,
{
    type Message = M;
    type Argument = Arg;
    type Actor = A;
    fn new(&mut self, ctx: ActorContext<Self::Message>, arg: Self::Argument) -> Self::Actor {
        (self)(ctx, arg)
    }
}

impl<M, Arg1, Arg2, A> NewActor for fn(ActorContext<M>, Arg1, Arg2) -> A
    where A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2);
    type Actor = A;
    fn new(&mut self, ctx: ActorContext<Self::Message>, arg: Self::Argument) -> Self::Actor {
        (self)(ctx, arg.0, arg.1)
    }
}

impl<M, Arg1, Arg2, Arg3, A> NewActor for fn(ActorContext<M>, Arg1, Arg2, Arg3) -> A
    where A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3);
    type Actor = A;
    fn new(&mut self, ctx: ActorContext<Self::Message>, arg: Self::Argument) -> Self::Actor {
        (self)(ctx, arg.0, arg.1, arg.2)
    }
}

impl<M, Arg1, Arg2, Arg3, Arg4, A> NewActor for fn(ActorContext<M>, Arg1, Arg2, Arg3, Arg4) -> A
    where A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3, Arg4);
    type Actor = A;
    fn new(&mut self, ctx: ActorContext<Self::Message>, arg: Self::Argument) -> Self::Actor {
        (self)(ctx, arg.0, arg.1, arg.2, arg.3)
    }
}

impl<M, Arg1, Arg2, Arg3, Arg4, Arg5, A> NewActor for fn(ActorContext<M>, Arg1, Arg2, Arg3, Arg4, Arg5) -> A
    where A: Actor,
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3, Arg4, Arg5);
    type Actor = A;
    fn new(&mut self, ctx: ActorContext<Self::Message>, arg: Self::Argument) -> Self::Actor {
        (self)(ctx, arg.0, arg.1, arg.2, arg.3, arg.4)
    }
}

/// The main actor trait.
///
/// Effectively an `Actor` is a `Future` which returns a `Result<(), Error>`,
/// where `Error` is defined on the trait. That is why there is a blanket
/// implementation for all `Future`s with a `Result<(), Error>` as `Output`
/// type.
///
/// The easiest way to implement this by using an async function, see the
/// [module level] documentation.
///
/// [module level]: index.html
///
/// # Panics
///
/// Because this is basically a `Future` it also shares it's characteristics,
/// including it's unsafety. Please read the `Future` documentation when
/// implementing or using this by hand.
pub trait Actor {
    /// An error the actor can return to it's supervisor. This error will be
    /// considered terminal for this actor and should **not** not be an error of
    /// regular processing of a message.
    ///
    /// How to process non-terminal errors that happen during regular processing
    /// of messages is up to the actor.
    type Error;

    /// Try to poll this actor.
    ///
    /// This is basically the same as calling `Future::poll`.
    ///
    /// # Panics
    ///
    /// Just like with futures polling an after it returned `Poll::Ready` may
    /// cause undefined behaviour, including but not limited to panicking.
    fn try_poll(self: Pin<&mut Self>, waker: &LocalWaker) -> Poll<Result<(), Self::Error>>;
}

impl<Fut, E> Actor for Fut
    where Fut: Future<Output = Result<(), E>>
{
    type Error = E;

    fn try_poll(self: Pin<&mut Self>, waker: &LocalWaker) -> Poll<Result<(), Self::Error>> {
        self.poll(waker)
    }
}

/// The implementation behind [`actor_factory`].
///
/// [`actor_factory`]: fn.actor_factory.html
pub struct ActorFactory<NA, M, Arg> {
    new_actor: NA,
    _phantom: PhantomData<(M, Arg)>,
}

impl<NA, M, Arg, A> NewActor for ActorFactory<NA, M, Arg>
    where NA: FnMut(ActorContext<M>, Arg) -> A,
          A: Actor,
{
    type Message = M;
    type Argument = Arg;
    type Actor = A;

    fn new(&mut self, ctx: ActorContext<Self::Message>, arg: Self::Argument) -> Self::Actor {
        (self.new_actor)(ctx, arg)
    }
}

impl<NA, M, Arg> Copy for ActorFactory<NA, M, Arg>
    where NA: Copy,
{
}

impl<NA, M, Arg> Clone for ActorFactory<NA, M, Arg>
    where NA: Clone,
{
    fn clone(&self) -> ActorFactory<NA, M, Arg> {
        ActorFactory {
            new_actor: self.new_actor.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<NA, M, Arg> fmt::Debug for ActorFactory<NA, M, Arg> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorFactory")
            .finish()
    }
}

// This is safe because we only really own `NA`.
unsafe impl<NA, M, Arg> Send for ActorFactory<NA, M, Arg>
    where NA: Send
{
}

// This is safe because we only really own `NA`.
unsafe impl<NA, M, Arg> Sync for ActorFactory<NA, M, Arg>
    where NA: Sync
{
}

/// Implement [`NewActor`] by means of a function.
///
/// The easiest and recommended way to use this is via async functions, see the
/// example below.
///
/// [`NewActor`]: trait.NewActor.html
///
/// # Example
///
/// Using an async function.
///
/// ```
/// #![feature(async_await, futures_api, never_type)]
///
/// use heph::actor::{ActorContext, actor_factory};
///
/// async fn actor(mut ctx: ActorContext<()>, _arg: ()) -> Result<(), !> {
///     println!("Hello from the actor!");
///     Ok(())
/// }
///
/// // Our `NewActor` implementation that returns our actor.
/// let new_actor = actor_factory(actor);
/// #
/// # fn use_new_actor<NA: heph::actor::NewActor>(new_actor: NA) { }
/// # use_new_actor(new_actor);
/// ```
pub const fn actor_factory<NA, M, Arg, A>(new_actor: NA) -> ActorFactory<NA, M, Arg>
    where NA: FnMut(ActorContext<M>, Arg) -> A,
          A: Actor,
{
    ActorFactory {
        new_actor,
        _phantom: PhantomData,
    }
}
