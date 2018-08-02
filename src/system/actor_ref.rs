//! Module containing the `ActorRef`.

use std::fmt;

use crate::system::MailBox;
use crate::system::error::{SendError, SendErrorReason};
use crate::util::WeakShared;

/// A reference to an actor inside a [`ActorSystem`].
///
/// This reference can be used to send messages to the actor. To share this
/// reference simply clone it.
///
/// [`ActorSystem`]: struct.ActorSystem.html
///
/// # Examples
///
/// Cloning an actor reference.
///
// FIXME: causes an iternal compiler error.
/// ```ignore
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use actor::actor::{ActorContext, actor_factory};
/// use actor::system::{ActorSystemBuilder, ActorOptions};
///
/// async fn actor(mut ctx: ActorContext<String>, _: ()) -> Result<(), !> {
///     loop {
///         let msg = await!(ctx.receive());
///         println!("got message: {}", msg);
///     }
/// }
///
/// // Create `ActorSystem` and add our actor to it.
/// let mut actor_system = ActorSystemBuilder::default().build().unwrap();
/// let new_actor = actor_factory(actor);
/// let mut actor_ref = actor_system.add_actor(new_actor, (), ActorOptions::default());
///
/// // Send a message to the actor.
/// actor_ref.send("Hello".to_owned());
///
/// // To create another `ActorRef` we can simply clone the first one.
/// let mut second_actor_ref = actor_ref.clone();
/// second_actor_ref.send("World".to_owned());
/// ```
pub struct ActorRef<M> {
    /// The inbox of the `Actor`, owned by the `ActorContext`.
    inbox: WeakShared<MailBox<M>>,
}

impl<M> ActorRef<M> {
    /// Create a new `ActorRef` with a shared mailbox.
    pub(crate) const fn new(inbox: WeakShared<MailBox<M>>) -> ActorRef<M> {
        ActorRef {
            inbox,
        }
    }

    /// Send a message to the actor.
    ///
    /// # Examples
    ///
    /// Using an enum as message type.
    ///
    // FIXME: causes an iternal compiler error.
    /// ```ignore
    /// #![feature(async_await, await_macro, futures_api, never_type)]
    ///
    /// use actor::actor::{ActorContext, actor_factory};
    /// use actor::system::{ActorSystemBuilder, ActorOptions};
    ///
    /// async fn actor(mut ctx: ActorContext<Message>, _: ()) -> Result<(), !> {
    ///     loop {
    ///         let msg = await!(ctx.receive());
    ///         println!("got message: {:?}", msg);
    ///     }
    /// }
    ///
    /// // The message type for the actor.
    /// //
    /// // Using an enum we can allow a single actor to handle multiple types of
    /// // messages.
    /// #[derive(Debug)]
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
    /// // Create `ActorSystem` and add our actor to it.
    /// let mut actor_system = ActorSystemBuilder::default().build().unwrap();
    /// let new_actor = actor_factory(actor);
    /// let mut actor_ref = actor_system.add_actor(new_actor, (), ActorOptions::default());
    ///
    /// // Now we can use the reference to send the actor a message, without
    /// // having to use `Message` we can just use `String`.
    /// actor_ref.send("Hello world".to_owned());
    /// ```
    pub fn send<Msg>(&mut self, msg: Msg) -> Result<(), SendError<Msg>>
        where Msg: Into<M>,
    {
        match self.inbox.upgrade() {
            Some(mut inbox) => inbox.borrow_mut().deliver(msg),
            None => Err(SendError {
                message: msg,
                reason: SendErrorReason::ActorShutdown,
            }),
        }
    }
}

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> ActorRef<M> {
        ActorRef {
            inbox: self.inbox.clone(),
        }
    }
}

impl<M> fmt::Debug for ActorRef<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorRef")
            .finish()
    }
}
