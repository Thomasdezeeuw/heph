//! Module containing the `ActorRef`.

use std::rc::Rc;

use system::error::SendError;
use super::SharedMailbox;

/// A reference to an actor inside a running [`ActorSystem`].
///
/// This reference can be used to send messages to the actor. To share this
/// reference simply clone it.
///
/// [`ActorSystem`]: struct.ActorSystem.html
///
/// # Examples
///
/// ```
/// # extern crate actor;
/// # extern crate futures_core;
/// #
/// # use actor::actor::actor_fn;
/// # use actor::system::ActorSystemBuilder;
/// # use actor::system::ActorOptions;
/// # use futures_core::future::{FutureResult, ok};
/// #
/// // Create `ActorSystem` and `Actor`, etc.
/// # let mut actor_system = ActorSystemBuilder::default().build().unwrap();
/// # let actor = actor_fn(|_: ()| -> FutureResult<(), ()> { ok(()) });
///
/// let actor_ref = actor_system.add_actor(actor, ActorOptions::default())
///     .expect("unable to add actor to actor system");
///
/// // To create another `ActorRef` we can simply clone the first one.
/// let second_actor_ref = actor_ref.clone();
/// ```
#[derive(Debug)]
pub struct ActorRef<M> {
    /// The inbox of the `Actor`, owned by the `ActorProcess`.
    inbox: SharedMailbox<M>,
}

impl<M> ActorRef<M> {
    /// Create a new `ActorRef` with a shared mailbox.
    pub(super) fn new(inbox: SharedMailbox<M>) -> ActorRef<M> {
        ActorRef {
            inbox,
        }
    }

    /// Send a message to the actor.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate actor;
    /// # extern crate futures_core;
    /// #
    /// # use actor::actor::actor_fn;
    /// # use actor::system::ActorSystemBuilder;
    /// # use actor::system::ActorOptions;
    /// # use futures_core::future::{FutureResult, ok};
    /// #
    /// // The message type for the actor.
    /// //
    /// // Using an enum we can allow a single actor to handle multiple types of
    /// // messages.
    /// enum Message {
    ///     String(String),
    ///     Number(usize),
    /// }
    ///
    /// // Implementing `From` for the message allows us to just pass a
    /// // `String`, rather then a `Message::String`.
    /// impl From<String> for Message {
    ///     fn from(str: String) -> Message {
    ///         Message::String(str)
    ///     }
    /// }
    ///
    /// // Create `ActorSystem` and `Actor`, etc.
    /// # let mut actor_system = ActorSystemBuilder::default().build().unwrap();
    /// # let actor = actor_fn(|_: Message| -> FutureResult<(), ()> { ok(()) });
    ///
    /// let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default())
    ///     .expect("unable to add actor to actor system");
    ///
    /// // Now we can use the reference to send the actor a message, without
    /// // having to use `Message` we can just use `String`.
    /// actor_ref.send("Hello world".to_owned());
    /// ```
    pub fn send<Msg>(&mut self, msg: Msg) -> Result<(), SendError<M>>
        where Msg: Into<M>,
    {
        self.inbox.borrow_mut().deliver(msg.into())
    }
}

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> ActorRef<M> {
        ActorRef {
            inbox: Rc::clone(&self.inbox),
        }
    }
}
