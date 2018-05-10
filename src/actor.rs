//! The module with the [`Actor`] and [`NewActor`] trait definitions.
//!
//! All actors must implement the [`Actor`] trait, which defines how an actor
//! handles messages. However the system needs a way to create (and recreate)
//! these actors, which is defined in the [`NewActor`] trait. Helper structs are
//! provided to easily implement this trait, see [`ActorFactory`] and
//! [`ReusableActorFactory`].
//!
//! [`Actor`]: trait.Actor.html
//! [`NewActor`]: trait.NewActor.html
//! [`ActorFactory`]: struct.ActorFactory.html
//! [`ReusableActorFactory`]: struct.ReusableActorFactory.html

use std::marker::PhantomData;
use std::mem;

use futures_core::Future;

/// The main actor trait, which defines how an actor handles messages.
///
/// # Lifetime `'a`
///
/// The trait contains a lifetime `'a`, which is the lifetime for the object
/// that implements the `Actor` trait. However is also used in the `Future` type
/// to allow the future to reference the actor.
///
/// # Notes
///
/// When running into lifetime problems using a `Future` that references the
/// actor try using the `'a` lifetime manually in the `handle` function.
pub trait Actor<'a> {
    /// The user defined message that this actor can handle.
    ///
    /// Use an enum to allow an actor to handle multiple types of messages.
    type Message;

    /// An error the actor can return to it's supervisor. This error will be
    /// considered terminal for this actor and should **not** not be an error of
    /// regular processing of a message.
    ///
    /// How to process non-terminal errors that happen during regular processing
    /// of messages is up to the user.
    type Error;

    /// The future returned by the actor to handle a message.
    ///
    /// The returned item is discarded, while the returned error is passed to
    /// the actor's supervisor, if any.
    // TODO: link to supervisor.
    type Future: Future<Item = (), Error = Self::Error> + 'a;

    /// Handle a message, the core of this trait.
    ///
    /// # Note
    ///
    /// The returned future will be completed before another message is handled
    /// by this actor, effectively blocking this actor until the future is
    /// completed. If the returned future does any blocking operations, e.g.
    /// I/O, it's recommended to make an actor to handle that blocking
    /// operation. For example a new actor per request to handle.
    fn handle(&'a mut self, message: Self::Message) -> Self::Future;

    /// The method that will be called once the actor is created, but not yet
    /// has received it's first message.
    ///
    /// The default is to do nothing.
    fn pre_start(&'a mut self) { }

    /// The method that will be called after the actor received it's final
    /// message, just before the actor is dropped.
    ///
    /// The default is to do nothing.
    ///
    /// # Note
    ///
    /// Sending messages to other actors from this functions will have no
    /// effect. This method is only called when the system is being shutdown,
    /// save for a restart of the actor, and other actors will be (or are
    /// already) stopped as well.
    fn post_stop(&'a mut self) { }

    /// The method that will be called once an actor will be restarted, but just
    /// before actually stopping the actor.
    ///
    /// The default is to call the [`post_stop`] function.
    ///
    /// [`post_stop`]: trait.Actor.html#method.post_stop
    fn pre_restart(&'a mut self) {
        self.post_stop();
    }

    /// The method that will be called once an actor is restarted, but just
    /// before it will accept it's first message.
    ///
    /// The default is to call the [`pre_start`] function.
    ///
    /// # Note
    ///
    /// Contrary to the `post_stop` method, this method is allowed to send
    /// messages to other actors since the actor is being restarted but the
    /// system isn't shutting down.
    ///
    /// [`pre_start`]: trait.Actor.html#method.pre_start
    fn post_restart(&'a mut self) {
        self.pre_start();
    }
}

/// The trait that defines how to create a new actor.
///
/// An easy way to implement this by means of a function is to use
/// [`ActorFactory`] or [`ReusableActorFactory`].
///
/// [`ActorFactory`]: struct.ActorFactory.html
/// [`ReusableActorFactory`]: struct.ReusableActorFactory.html
///
/// # Lifetimes
///
/// This trait has two lifetimes `'n` and `'a`. `'n` is the lifetime for the
/// object that implements the `NewActor` trait, while `'a` defines a separate
/// lifetime for the object that implements the `Actor` trait. These lifetimes
/// are not bound to each other.
pub trait NewActor<'n, 'a> {
    /// The type of the actor, see [`Actor`](trait.Actor.html).
    type Actor: Actor<'a> + 'a;

    /// The initial item the actor will be created with.
    ///
    /// The could for example be a TCP connection the actor is responsible for.
    type Item;

    /// The method that gets called to create a new actor.
    fn new(&'n mut self, item: Self::Item) -> Self::Actor;

    /// Reuse an already allocated actor.
    ///
    /// The default implementation will create a new actor (by calling
    /// [`new`](trait.NewActor.html#tymethod.new)) and replace `old_actor` with
    /// it.
    ///
    /// This is a performance optimization to allow the allocations of an actor
    /// to be reused.
    fn reuse(&'n mut self, old_actor: &mut Self::Actor, item: Self::Item) {
        mem::replace(old_actor, self.new(item));
    }
}

// TODO: change factories to:
// ```
// pub struct ActorFactory<N>(pub N).
// pub struct ReusableActorFactory<N, R>(pub N, pub R).
// ```
// Currently doesn't work due to unused type parameter error in `NewActor`
// implementations.

/// A construct that allows [`NewActor`] to be implemented by means of a
/// function. If a custom [reuse] function is needed see
/// [`ReusableActorFactory`].
///
/// See the [`actor_factory`] function to create a `ActorFactory`. Alternatively
/// the [`From`] implementation can be used.
///
/// [`NewActor`]: trait.NewActor.html
/// [reuse]: trait.NewActor.html#method.reuse
/// [`ReusableActorFactory`]: struct.ReusableActorFactory.html
/// [`actor_factory`]: fn.actor_factory.html
/// [`From`]: struct.ActorFactory.html#impl-From<N>
///
/// # Example
///
/// ```
/// # extern crate actor;
/// # extern crate futures_core;
/// # use actor::actor::{Actor, NewActor};
/// # use futures_core::Future;
/// use actor::actor::actor_factory;
///
/// // Our actor that implements the `Actor` trait.
/// struct MyActor;
///
/// # impl<'a> Actor<'a> for MyActor {
/// #    type Message = ();
/// #    type Error = ();
/// #    type Future = Box<Future<Item = (), Error = ()>>;
/// #    fn handle(&'a mut self, _: ()) -> Self::Future { unimplemented!(); }
/// # }
/// #
/// # fn main() {
/// // Our `NewActor` implementation that returns our actor.
/// let new_actor = actor_factory(|_: ()| MyActor);
/// #
/// # fn use_new_actor<'n, 'a, A: NewActor<'n, 'a>>(new_actor: A) { }
/// # use_new_actor(new_actor);
/// # }
/// ```
#[derive(Debug)]
pub struct ActorFactory<N, I, A>{
    new_actor: N,
    _phantom: PhantomData<(I, A)>,
}

impl<'n, 'a, N, I, A> NewActor<'n, 'a> for ActorFactory<N, I, A>
    where N: FnMut(I) -> A,
          A: Actor<'a> + 'a,
{
    type Actor = A;
    type Item = I;
    fn new(&'n mut self, item: I) -> A {
        (self.new_actor)(item)
    }
}

impl<'a, N, I, A> From<N> for ActorFactory<N, I, A>
    where N: FnMut(I) -> A,
          A: Actor<'a>,
{
    fn from(new_actor: N) -> Self {
        actor_factory(new_actor)
    }
}

/// Create a new [`ActorFactory`].
///
/// [`ActorFactory`]: struct.ActorFactory.html
pub fn actor_factory<'a, N, I, A>(new_actor: N) -> ActorFactory<N, I, A>
    where N: FnMut(I) -> A,
          A: Actor<'a>,
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
/// `ReusableActorFactory`. Alternatively the [`From`] implementation can be
/// used.
///
/// [`NewActor`]: trait.NewActor.html
/// [`ActorFactory`]: struct.ActorFactory.html
/// [`reusable_actor_factory`]: fn.reusable_actor_factory.html
/// [`From`]: struct.ReusableActorFactory.html#impl-From<(N%2C R)>
///
/// # Example
///
/// ```
/// # extern crate actor;
/// # extern crate futures_core;
/// # use actor::actor::{Actor, NewActor};
/// # use futures_core::Future;
/// use actor::actor::reusable_actor_factory;
///
/// // Our actor that implements the `Actor` trait.
/// struct MyActor;
///
/// # impl<'a> Actor<'a> for MyActor {
/// #    type Message = ();
/// #    type Error = ();
/// #    type Future = Box<Future<Item = (), Error = ()>>;
/// #    fn handle(&mut self, _: ()) -> Self::Future { unimplemented!(); }
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
/// # fn use_new_actor<'n, 'a, A: NewActor<'n, 'a>>(new_actor: A) { }
/// # use_new_actor(new_actor);
/// # }
/// ```
#[derive(Debug)]
pub struct ReusableActorFactory<N, R, I, A> {
    new_actor: N,
    reuse_actor: R,
    _phantom: PhantomData<(I, A)>,
}

impl<'n, 'a, N, R, I, A> NewActor<'n, 'a> for ReusableActorFactory<N, R, I, A>
    where N: FnMut(I) -> A,
          R: FnMut(&mut A, I),
          A: Actor<'a> + 'a,
{
    type Actor = A;
    type Item = I;
    fn new(&'n mut self, item: I) -> A {
        (self.new_actor)(item)
    }
    fn reuse(&'n mut self, old_actor: &mut A, item: I) {
        (self.reuse_actor)(old_actor, item)
    }
}

impl<'a, N, R, I, A> From<(N, R)> for ReusableActorFactory<N, R, I, A>
    where N: FnMut(I) -> A,
          R: FnMut(&mut A, I),
          A: Actor<'a>,
{
    fn from(args: (N, R)) -> Self {
        reusable_actor_factory(args.0, args.1)
    }
}

/// Create a new [`ReusableActorFactory`].
///
/// [`ReusableActorFactory`]: struct.ReusableActorFactory.html
pub fn reusable_actor_factory<'a, N, R, I, A>(new_actor: N, reuse_actor: R) -> ReusableActorFactory<N, R, I, A>
    where N: FnMut(I) -> A,
          R: FnMut(&mut A, I),
          A: Actor<'a>,
{
    ReusableActorFactory {
        new_actor,
        reuse_actor,
        _phantom: PhantomData,
    }
}
