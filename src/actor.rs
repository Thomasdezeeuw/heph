// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

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

use std::mem;

use futures::IntoFuture;

/// The main actor trait, which defines how an actor handles messages.
pub trait Actor {
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
    type Future: IntoFuture<Item = (), Error = Self::Error>;

    /// Handle a message, the core of this trait.
    ///
    /// # Note
    ///
    /// The returned future will be completed before another message is handled
    /// by this actor, effectively blocking this actor until the future is
    /// completed. If the returned future does any blocking operations, e.g.
    /// I/O, it's recommended to make an actor to handle that blocking
    /// operation. For example a new actor per request to handle.
    fn handle(&mut self, message: Self::Message) -> Self::Future;

    /// The method that will be called once the actor is created, but not yet
    /// has received it's first message.
    ///
    /// The default is to do nothing.
    fn pre_start(&mut self) { }

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
    fn post_stop(&mut self) { }

    /// The method that will be called once an actor will be restarted, but just
    /// before actually stopping the actor.
    ///
    /// The default is to call the [`post_stop`] function.
    ///
    /// [`post_stop`]: trait.Actor.html#method.post_stop
    fn pre_restart(&mut self) {
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
    fn post_restart(&mut self) {
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
pub trait NewActor {
    /// The type of the actor, see [`Actor`](trait.Actor.html).
    type Actor: Actor;

    /// The method that gets called to create a new actor.
    fn new(&self) -> Self::Actor;

    /// Reuse an already allocated actor. The default implementation will create
    /// a new actor (by calling [`new`](trait.NewActor.html#tymethod.new)) and
    /// replace `old_actor` with it.
    ///
    /// This is a performance optimization to allow the allocations of an actor
    /// to be reused.
    fn reuse(&self, old_actor: &mut Self::Actor) {
        mem::replace(old_actor, self.new());
    }
}

/// A contruct that allows [`NewActor`] to be implemented by means of a
/// function. If a custom [reuse] function is needed see [`ReusableActorFactory`].
///
/// # Example
///
/// ```
///
/// # extern crate actor;
/// # extern crate futures;
/// # use actor::actor::{Actor, NewActor};
/// # use futures::Future;
/// use actor::actor::ActorFactory;
///
/// // Our actor that implements the `Actor` trait.
/// struct MyActor;
///
/// # impl Actor for MyActor {
/// #    type Message = ();
/// #    type Error = ();
/// #    type Future = Box<Future<Item = (), Error = ()>>;
/// #    fn handle(&mut self, _: ()) -> Self::Future { unimplemented!(); }
/// # }
/// #
/// # fn main() {
/// // Our `NewActor` implementation that returns our actor.
/// let new_actor = ActorFactory(|| MyActor);
/// #
/// # fn use_new_actor<A: NewActor>(new_actor: A) { }
/// # use_new_actor(new_actor);
/// # }
/// ```
///
/// [`NewActor`]: trait.NewActor.html
/// [reuse]: trait.NewActor.html#method.reuse
/// [`ReusableActorFactory`]: struct.ReusableActorFactory.html
pub struct ActorFactory<N, A>(pub N)
    where N: Fn() -> A,
          A: Actor;

impl<N, A> NewActor for ActorFactory<N, A>
    where N: Fn() -> A,
          A: Actor,
{
    type Actor = A;
    fn new(&self) -> Self::Actor {
        (self.0)()
    }
}

/// A contruct that allows [`NewActor`] to be implemented by means of a
/// function, including the reuse of an actor.
///
/// # Example
///
/// ```
/// # extern crate actor;
/// # extern crate futures;
/// # use actor::actor::{Actor, NewActor};
/// # use futures::Future;
/// use actor::actor::ReusableActorFactory;
///
/// // Our actor that implements the `Actor` trait.
/// struct MyActor;
///
/// # impl Actor for MyActor {
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
/// let new_actor = ReusableActorFactory(|| MyActor, |actor| actor.reset());
/// #
/// # fn use_new_actor<A: NewActor>(new_actor: A) { }
/// # use_new_actor(new_actor);
/// # }
/// ```
///
/// [`NewActor`]: trait.NewActor.html
pub struct ReusableActorFactory<N, R, A>(pub N, pub R)
    where N: Fn() -> A,
          R: Fn(&mut A),
          A: Actor;

impl<N, R, A> NewActor for ReusableActorFactory<N, R, A>
    where N: Fn() -> A,
          R: Fn(&mut A),
          A: Actor,
{
    type Actor = A;
    fn new(&self) -> Self::Actor {
        (self.0)()
    }
    fn reuse(&self, old_actor: &mut Self::Actor) {
        (self.1)(old_actor)
    }
}
