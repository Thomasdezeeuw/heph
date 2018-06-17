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

use futures_core::{Future, Async, Poll};
use futures_core::task::Context;

/// The main actor trait, which defines how an actor handles messages.
///
/// The `Actor` trait is basically a special version of a `Future`, one which
/// can be given more work. When a message arrives for the actor, the `handle`
/// method will be called to allow the actor to process the message. But, just
/// like with futures, it is possible that message can't be processed without
/// blocking, something we don't want.
///
/// To prevent an actor from blocking the process it should return
/// `Async::NotReady`, just like a future. And just like a future it should make
/// sure it's scheduled again at a later date, see the `Future` trait for more.
/// Once the actor is scheduled again, this time `Future`'s `poll` method will be
/// called, instead of `handle`.
///
/// Once `Async::Ready` is returned the actor must ready to receive another
/// message, `handle` will be called again and the entire process described
/// above is repeated.
///
/// The main difference with a regular future is that the actor will be given
/// more work (via a message) once the previous message is processed. Calling
/// `poll` on a future after it returned `Async::Ready` causes undefined
/// behaviour, the same is true for an actor. **However** `handle` should be
/// callable when another message is ready for the actor, after which `poll`
/// should also be callable (if `handle` didn't return `Async::Ready`).
///
/// In short an `Actor` is a `Future` which one can give more work, via
/// messages.
pub trait Actor: Future<Item = ()> {
    /// The user defined message that this actor can handle.
    ///
    /// Use an enum to allow an actor to handle multiple types of messages.
    type Message;

    /// Handle a message, the core of this trait.
    fn handle(&mut self, ctx: &mut Context, message: Self::Message) -> Poll<(), Self::Error>;
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
/// use actor::actor::{Actor, actor_fn};
///
/// # fn main() {
/// // Our `Actor` implementation.
/// let actor = actor_fn(|msg: String| -> Result<(), ()> {
///     println!("got a message: {}", msg);
///     Ok(())
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

impl<F, M, E> Future for ActorFn<F, M>
    where F: FnMut(M) -> Result<(), E>,
{
    type Item = ();
    type Error = E;
    /// Calls to poll will always return `Async::Ready`.
    fn poll(&mut self, _: &mut Context) -> Poll<(), Self::Error> {
        Ok(Async::Ready(()))
    }
}

impl<F, M, E> Actor for ActorFn<F, M>
    where F: FnMut(M) -> Result<(), E>,
{
    type Message = M;
    fn handle(&mut self, _: &mut Context, message: Self::Message) -> Poll<(), Self::Error> {
        (self.func)(message).map(|()| Async::Ready(()))
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
    where F: FnMut(M) -> Result<(), E>,
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
/// # extern crate actor;
/// # extern crate futures_core;
/// # use actor::actor::{Actor, NewActor};
/// # use futures_core::{Future, Poll};
/// # use futures_core::task::Context;
/// use actor::actor::actor_factory;
///
/// // Our actor that implements the `Actor` trait.
/// struct MyActor;
///
/// # impl Future for MyActor {
/// #     type Item = ();
/// #     type Error = ();
/// #     fn poll(&mut self, _: &mut Context) -> Poll<(), ()> { unimplemented!(); }
/// # }
/// #
/// # impl Actor for MyActor {
/// #     type Message = String;
/// #     fn handle(&mut self, _: &mut Context, _: String) -> Poll<(), ()> { unimplemented!(); }
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
pub const fn actor_factory<'a, N, I, A>(new_actor: N) -> ActorFactory<N, I>
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
/// # extern crate actor;
/// # extern crate futures_core;
/// # use actor::actor::{Actor, NewActor};
/// # use futures_core::{Future, Poll};
/// # use futures_core::task::Context;
/// use actor::actor::reusable_actor_factory;
///
/// // Our actor that implements the `Actor` trait.
/// struct MyActor;
///
/// # impl Future for MyActor {
/// #     type Item = ();
/// #     type Error = ();
/// #     fn poll(&mut self, _: &mut Context) -> Poll<(), ()> { unimplemented!(); }
/// # }
/// #
/// # impl Actor for MyActor {
/// #     type Message = String;
/// #     fn handle(&mut self, _: &mut Context, _: String) -> Poll<(), ()> { unimplemented!(); }
/// # }
/// #
/// impl MyActor {
///     fn reset(&mut self) { /* Reset our actor. */ }
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
