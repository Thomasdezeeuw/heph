// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

//! All actors must implement [`Actor`], the trait that defines how an actor
//! handles messages. However the system needs a way to create these actors,
//! which is defined in the [`NewActor`] trait. A helper struct is provided to
//! easily implement this trait, see [`ActorFactory`].
//!
//! [`Actor`]: trait.Actor.html
//! [`NewActor`]: trait.NewActor.html
//! [`ActorFactory`]: struct.ActorFactory.html

use std::mem;

use futures::Future;

pub trait Actor {
    /// The message that actor can handle.
    ///
    /// Use an enum to allow an actor to handle multiple types of messages.
    type Message;

    /// An error the actor can return to it's supervisor. This error will be
    /// consider terminal for this actor and should *not* not be an error of
    /// regular processing of a message.
    ///
    /// How to process non-terminal errors that happen in regular processing is
    /// up to the user.
    // TODO: give advance about how to handle non-terminal errors.
    type Error;

    /// The future returned by the actor to handle a message.
    ///
    /// The returned item is discarded, while the returned error is passed to
    /// the actor's supervisor.
    type Future: Future<Item = (), Error = Self::Error>;

    /// Handle a message, the core of this trait.
    ///
    /// # Note
    ///
    /// The returned future will be completed before another message is handled
    /// by this actor, effectively blocking this actor until this actor until
    /// it's completed. If the returned future does any I/O or other blocking
    /// operations it's recommended to make an actor specific to that blocking
    /// operation, e.g. a unqiue actor per request to handle the reading and
    /// writing of the requests/responses.
    fn handle(&mut self, message: Self::Message) -> Self::Future;

    // TODO: determine and doc the actor's lifecycle.
    //
    // TODO: describe when an actor will be restarted; when it returns an actor
    // and the supervisor says so.

    /// The method that will be called once the actor is created, but not yet
    /// has received it's first message.
    ///
    /// The default is to do nothing.
    fn pre_start(&mut self) { }

    /// The method that will be called after the actor received it's final
    /// message, just before it's dropped.
    ///
    /// The default is to do nothing.
    fn post_stop(&mut self) { }

    /// The method that will be called once an actor will be restarted, but just
    /// before actually stopping the actor.
    ///
    /// The default is to call `post_stop` function.
    fn pre_restart(&mut self) {
        self.post_stop();
    }

    /// The method that will be called once an actor is restarted, but just
    /// before it will accept it's first message.
    ///
    /// The default is to call `pre_start` function.
    fn post_restart(&mut self) {
        self.pre_start();
    }
}
