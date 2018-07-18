//! The module with the `Actor` and `NewActor` trait definitions.
//!
//! All actors must implement the [`Actor`] trait, which defines how an actor
//! handles messages. The easiest way to implement this trait is to use the
//! [`ActorFn`] helper struct.
//!
//! However the system needs a way to create (and recreate) these actors, which
//! is defined in the [`NewActor`] trait. Helper structs are provided to easily
//! implement this trait, see [`ActorFactory`] and [`ReusableActorFactory`].
//!
//! [`Actor`]: trait.Actor.html
//! [`NewActor`]: trait.NewActor.html
//! [`ActorFn`]: struct.ActorFn.html
//! [`ActorFactory`]: struct.ActorFactory.html
//! [`ReusableActorFactory`]: struct.ReusableActorFactory.html

use std::{fmt, mem};
use std::marker::PhantomData;
use std::task::{Poll, Context, LocalWaker};

use crate::system::ActorSystemRef;

#[cfg(all(test, feature = "test"))]
mod tests;

/// The main actor trait, which defines how an actor handles messages.
///
/// The `Actor` trait is basically a special version of a `Future`, one which
/// can be given more work. When a message arrives for the actor, the `handle`
/// method will be called to allow the actor to process the message. But, just
/// like with futures, it is possible that message can't be processed without
/// blocking, something we don't want.
///
/// To prevent an actor from blocking the process it should return
/// `Poll::Pending`, just like a future. And just like a future it should make
/// sure it's scheduled again at a later date, see the `Future` trait for more.
/// Once the actor is scheduled again, this time the `poll` method will be
/// called, instead of `handle`, to continue handling the same message.
///
// TODO: the text references to futures for scheduling, maybe explain that here?
///
/// Once an actor is done handling a message it must decide whether it wants to
/// handle another message. To indicate the actor is done handing an message it
/// must return `Poll::Ready`. Actors that only handle a single message or item,
/// e.g. actor that handle a single connection, should return `Status::Complete`
/// to indicate that the actor has completed its tasks and can be removed from
/// the actor system. If however the actor needs to handle multiple message it
/// needs to return `Status::Ready` to indicate the actor is ready to handle
/// more message, which leaves it usable in the actor system.
///
/// If the actor returned `Status::Ready` it can receive another message,
/// `handle` will be called again and the entire process described above is
/// repeated.
///
/// The main difference with a regular future is that the actor will be given
/// more work (via a message) once the previous message is processed. Calling
/// `poll` on a future after it returned `Poll::Ready` causes undefined
/// behaviour, the same is true for an actor. Calling either `handle` or `poll`
/// after it returned `Status::Complete` is also undefined behaviour.
/// **However** `handle` should be callable when the actor previously returned
/// `Status::Ready` and another message is ready for the actor. `poll` should
/// always be called if it hasn't return `Poll::Ready` yet.
///
/// In short an `Actor` is a `Future` which one can give more work, via
/// messages.
pub trait Actor {
    /// The user defined message that this actor can handle.
    ///
    /// Use an enum to allow an actor to handle multiple types of messages.
    // TODO: say something about implementing `From` for the message, i.e.
    // provide an example.
    type Message;

    /// An error the actor can return to it's supervisor. This error will be
    /// considered terminal for this actor and should **not** not be an error of
    /// regular processing of a message.
    ///
    /// How to process non-terminal errors that happen during regular processing
    /// of messages is up to the user.
    type Error;

    /// Handle a message, the core of this trait.
    fn handle(&mut self, ctx: &mut ActorContext, msg: Self::Message) -> ActorResult<Self::Error>;

    /// Poll the actor to complete it's work on the current message.
    fn poll(&mut self, ctx: &mut ActorContext) -> ActorResult<Self::Error>;
}

/// The return type for various methods in the [`Actor`] trait.
///
/// This is returned by the `handle` and `poll` methods of the `Actor` trait. It
/// serves as a convenience to not having to write the entire type every time.
///
/// [`Actor`]: trait.Actor.html
// TODO: rename to AsyncActorResult, or ActorPollResult?
pub type ActorResult<E> = Poll<Result<Status, E>>;

/// The status of an actor.
///
/// See the [`Actor`] trait for more.
///
/// # Difference with `Poll`
///
/// The `Actor` trait used both `Poll` and this `Status` in the return type.
/// `Poll` is used as an indicator regarding the handling of a single message,
/// while `Status` is the indicator used regarding the entire actor. So if an
/// actor is not done handling a message (and `poll` should be called in the
/// future) it should return `Poll::Pending`. Once the message is handled it
/// should return `Poll::Ready` along with a status. `Status::Complete` should
/// be returned if the actor is done handling message and should be removed from
/// the actor system, while `Status::Ready` should be returned otherwise.
///
/// [`Actor`]: trait.Actor.html
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[must_use]
pub enum Status {
    /// The actor has completed all its work and is ready to shutdown.
    Complete,

    /// The actor has not yet completed all its work and is **not** ready to
    /// shutdown.
    Ready,
}

/// The context in which an actor is executed.
///
/// This can be used to get references to the actor itself, a possible sender of
/// the message or the system in which the actor is running.
#[derive(Debug)]
pub struct ActorContext {
    waker: LocalWaker,
    system_ref: ActorSystemRef,
}

impl ActorContext {
    /// Create a new `ActorContext`.
    pub(crate) const fn new(waker: LocalWaker, system_ref: ActorSystemRef) -> ActorContext {
        ActorContext {
            waker,
            system_ref,
        }
    }

    /// Create a new `ActorContext` that can be used in unit testing.
    ///
    /// # Notes
    ///
    /// The `Waker` implementation in the futures context is a noop.
    #[cfg(feature = "test")]
    pub fn test_ctx() -> ActorContext {
        use ::std::sync::Arc;
        use ::std::task::{local_waker, Wake};

        struct Waker;

        impl Wake for Waker {
            fn wake(_arc_self: &Arc<Self>) { }
        }

        ActorContext {
            waker: unsafe { local_waker(Arc::new(Waker)) },
            system_ref: ActorSystemRef::test_ref(),
        }
    }

    /// Get a context for executing a future.
    pub fn task_ctx(&mut self) -> Context {
        Context::new(&self.waker, &mut self.system_ref)
    }
}

// TODO: change ActorFn to:
// ```
// pub struct ActorFn<Fn>(pub Fn)
// ```
// Currently doesn't work due to unused type parameter error in `Actor`
// implementations.

/// Implement [`Actor`] by means of a function.
///
/// Note that this form of actor is very limited. It will have to handle message
/// in a single function call, without being able to make use of the `Future`s
/// ecosystem.
///
/// See the [`actor_fn`] function to create an `ActorFn`.
///
/// [`Actor`]: trait.Actor.html
/// [`actor_fn`]: fn.actor_fn.html
///
/// # Example
///
/// ```
/// # extern crate actor;
/// use actor::actor::{Actor, ActorContext, Status, actor_fn};
///
/// # fn main() {
/// // Our `Actor` implementation that prints a single message it receives and
/// // shuts down.
/// let actor = actor_fn(|_: &mut ActorContext, msg: String| -> Result<Status, ()> {
///     println!("got a message: {}", msg);
///     Ok(Status::Ready)
/// });
/// #
/// # fn use_actor<A: Actor>(actor: A) { }
/// # use_actor(actor);
/// # }
/// ```
pub struct ActorFn<F, M> {
    func: F,
    _phantom: PhantomData<M>,
}

impl<F, M, E> Actor for ActorFn<F, M>
    where F: FnMut(&mut ActorContext, M) -> Result<Status, E>,
{
    type Message = M;
    type Error = E;

    fn handle(&mut self, ctx: &mut ActorContext, msg: Self::Message) -> ActorResult<Self::Error> {
        Poll::Ready((self.func)(ctx, msg))
    }

    fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
        Poll::Ready(Ok(Status::Ready))
    }
}

impl<F, M> fmt::Debug for ActorFn<F, M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorFn")
            .finish()
    }
}

/// Create a new [`ActorFn`].
///
/// [`ActorFn`]: struct.ActorFn.html
pub const fn actor_fn<F, M, E>(func: F) -> ActorFn<F, M>
    where F: FnMut(&mut ActorContext, M) -> Result<Status, E>,
{
    ActorFn {
        func,
        _phantom: PhantomData,
    }
}

/// The trait that defines how to create a new actor.
///
/// An easy way to implement this by means of a function is to use
/// [`ActorFactory`] or [`ReusableActorFactory`].
///
/// [`ActorFactory`]: struct.ActorFactory.html
/// [`ReusableActorFactory`]: struct.ReusableActorFactory.html
pub trait NewActor {
    /// The type of the actor, see [`Actor`](trait.Actor.html).
    type Actor: Actor;

    /// The initial item the actor will be created with.
    ///
    /// The could for example be a TCP connection the actor is responsible for.
    type Item;

    /// The method that gets called to create a new actor.
    fn new(&mut self, item: Self::Item) -> Self::Actor;

    /// Reuse an already allocated actor.
    ///
    /// The default implementation will create a new actor (by calling
    /// [`new`](trait.NewActor.html#tymethod.new)) and replace `old_actor` with
    /// it.
    ///
    /// This is a performance optimization to allow the allocations of an actor
    /// to be reused.
    fn reuse(&mut self, old_actor: &mut Self::Actor, item: Self::Item) {
        drop(mem::replace(old_actor, self.new(item)));
    }
}

// TODO: change factories to:
// ```
// pub struct ActorFactory<N>(pub N)
// pub struct ReusableActorFactory<N, R>(pub N, pub R)
// ```
// Currently doesn't work due to unused type parameter error in `NewActor`
// implementations.

/// A construct that allows [`NewActor`] to be implemented by means of a
/// function. If a custom [reuse] function is needed see
/// [`ReusableActorFactory`].
///
/// See the [`actor_factory`] function to create an `ActorFactory`.
///
/// [`NewActor`]: trait.NewActor.html
/// [reuse]: trait.NewActor.html#method.reuse
/// [`ReusableActorFactory`]: struct.ReusableActorFactory.html
/// [`actor_factory`]: fn.actor_factory.html
///
/// # Example
///
/// ```
/// #![feature(arbitrary_self_types)]
/// #![feature(pin)]
///
/// # extern crate actor;
/// # use std::mem::PinMut;
/// # use actor::actor::{Actor, ActorContext, ActorResult, NewActor, Status};
/// use actor::actor::actor_factory;
///
/// // Our actor that implements the `Actor` trait.
/// struct MyActor;
///
/// # impl Actor for MyActor {
/// #     type Message = String;
/// #     type Error = ();
/// #     fn handle(&mut self, ctx: &mut ActorContext, msg: Self::Message) -> ActorResult<Self::Error> {
/// #         unimplemented!();
/// #     }
/// #     fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
/// #         unimplemented!();
/// #     }
/// # }
///
/// # fn main() {
/// // Our `NewActor` implementation that returns our actor.
/// let new_actor = actor_factory(|_: ()| MyActor);
/// #
/// # fn use_new_actor<A: NewActor>(new_actor: A) { }
/// # use_new_actor(new_actor);
/// # }
/// ```
pub struct ActorFactory<N, I> {
    new_actor: N,
    _phantom: PhantomData<I>,
}

impl<N, I, A> NewActor for ActorFactory<N, I>
    where N: FnMut(I) -> A,
          A: Actor,
{
    type Actor = A;
    type Item = I;
    fn new(&mut self, item: Self::Item) -> Self::Actor {
        (self.new_actor)(item)
    }
}

impl<N, I> fmt::Debug for ActorFactory<N, I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorFactory")
            .finish()
    }
}

/// Create a new [`ActorFactory`].
///
/// [`ActorFactory`]: struct.ActorFactory.html
pub const fn actor_factory<N, I, A>(new_actor: N) -> ActorFactory<N, I>
    where N: FnMut(I) -> A,
          A: Actor,
{
    ActorFactory {
        new_actor,
        _phantom: PhantomData,
    }
}

/// A construct that allows [`NewActor`] to be implemented by means of a
/// function, including the reuse of an actor. If no reuse function is desired
/// see [`ActorFactory`].
///
/// See the [`reusable_actor_factory`] function to create a
/// `ReusableActorFactory`.
///
/// [`NewActor`]: trait.NewActor.html
/// [`ActorFactory`]: struct.ActorFactory.html
/// [`reusable_actor_factory`]: fn.reusable_actor_factory.html
///
/// # Example
///
/// ```
/// #![feature(arbitrary_self_types)]
/// #![feature(pin)]
///
/// # extern crate actor;
/// # use std::mem::PinMut;
/// # use actor::actor::{Actor, ActorContext, ActorResult, NewActor, Status};
/// use actor::actor::reusable_actor_factory;
///
/// // Our actor that implements the `Actor` trait.
/// struct MyActor;
///
/// # impl Actor for MyActor {
/// #     type Message = String;
/// #     type Error = ();
/// #     fn handle(&mut self, ctx: &mut ActorContext, msg: Self::Message) -> ActorResult<Self::Error> {
/// #         unimplemented!();
/// #     }
/// #     fn poll(&mut self, _: &mut ActorContext) -> ActorResult<Self::Error> {
/// #         unimplemented!();
/// #     }
/// # }
///
/// impl MyActor {
///     pub fn reset(&mut self) {
///         // ...
///     }
/// }
///
/// # fn main() {
/// // Our `NewActor` implementation that returns our actor.
/// let new_actor = reusable_actor_factory(|_: ()| MyActor, |actor, _| actor.reset());
/// #
/// # fn use_new_actor<A: NewActor>(new_actor: A) { }
/// # use_new_actor(new_actor);
/// # }
/// ```
pub struct ReusableActorFactory<N, R, I> {
    new_actor: N,
    reuse_actor: R,
    _phantom: PhantomData<I>,
}

impl<N, R, I, A> NewActor for ReusableActorFactory<N, R, I>
    where N: FnMut(I) -> A,
          R: FnMut(&mut A, I),
          A: Actor,
{
    type Actor = A;
    type Item = I;
    fn new(&mut self, item: Self::Item) -> Self::Actor {
        (self.new_actor)(item)
    }
    fn reuse(&mut self, old_actor: &mut Self::Actor, item: Self::Item) {
        (self.reuse_actor)(old_actor, item)
    }
}

impl<N, R, I> fmt::Debug for ReusableActorFactory<N, R, I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ReusableActorFactory")
            .finish()
    }
}

/// Create a new [`ReusableActorFactory`].
///
/// [`ReusableActorFactory`]: struct.ReusableActorFactory.html
pub const fn reusable_actor_factory<N, R, I, A>(new_actor: N, reuse_actor: R) -> ReusableActorFactory<N, R, I>
    where N: FnMut(I) -> A,
          R: FnMut(&mut A, I),
          A: Actor,
{
    ReusableActorFactory {
        new_actor,
        reuse_actor,
        _phantom: PhantomData,
    }
}
