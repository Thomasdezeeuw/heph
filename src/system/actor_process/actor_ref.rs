//! Module containing the `ActorRef`.

use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use actor::Actor;
use system::actor_process::SharedMailbox;
use system::error::SendError;

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
/// #
/// # use actor::actor::{Status, actor_fn};
/// # use actor::system::ActorSystemBuilder;
/// # use actor::system::ActorOptions;
/// #
/// // Create `ActorSystem` and `Actor`, etc.
/// # let mut actor_system = ActorSystemBuilder::default().build().unwrap();
/// # let actor = actor_fn(|_, _: ()| -> Result<Status, ()> { Ok(Status::Ready) });
///
/// let actor_ref = actor_system.add_actor(actor, ActorOptions::default())
///     .unwrap_or_else(|err| panic!("unable to add actor to actor system: {}", err));
///
/// // To create another `ActorRef` we can simply clone the first one.
/// let second_actor_ref = actor_ref.clone();
/// ```
pub struct ActorRef<A>
    where A: Actor,
{
    /// The inbox of the `Actor`, owned by the `ActorProcess`.
    inbox: SharedMailbox<A::Message>,
    _phantom: PhantomData<A>,
}

impl<A> ActorRef<A>
    where A: Actor,
{
    /// Create a new `ActorRef` with a shared mailbox.
    pub(super) const fn new(inbox: SharedMailbox<A::Message>) -> ActorRef<A> {
        ActorRef {
            inbox,
            _phantom: PhantomData,
        }
    }

    /// Send a message to the actor.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate actor;
    /// #
    /// # use actor::actor::{Status, actor_fn};
    /// # use actor::system::ActorSystemBuilder;
    /// # use actor::system::ActorOptions;
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
    /// # let actor = actor_fn(|_, _: Message| -> Result<Status, ()> { Ok(Status::Ready) });
    ///
    /// let mut actor_ref = actor_system.add_actor(actor, ActorOptions::default())
    ///     .unwrap_or_else(|err| panic!("unable to add actor to actor system: {}", err));
    ///
    /// // Now we can use the reference to send the actor a message, without
    /// // having to use `Message` we can just use `String`.
    /// actor_ref.send("Hello world".to_owned());
    /// ```
    pub fn send<M>(&mut self, msg: M) -> Result<(), SendError<M>>
        where M: Into<A::Message>,
    {
        self.inbox.borrow_mut().deliver(msg)
    }
}

impl<A> Clone for ActorRef<A>
    where A: Actor,
{
    fn clone(&self) -> ActorRef<A> {
        ActorRef {
            inbox: Rc::clone(&self.inbox),
            _phantom: PhantomData,
        }
    }
}

impl<A> fmt::Debug for ActorRef<A>
    where A: Actor,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorRef")
            .finish()
    }
}
