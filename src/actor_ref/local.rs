//! Module containing the `LocalActorRef`.

use std::fmt;

use crate::actor_ref::MachineLocalActorRef;
use crate::error::{SendError, ErrorReason};
use crate::mailbox::MailBox;
use crate::util::WeakShared;

/// A reference to a local actor inside a [`ActorSystem`].
///
/// This is a reference to an actor running on the same thread as this reference
/// is on. This type does not implement `Send` or `Sync`, if this is needed this
/// reference can be [upgraded] to a [`MachineLocalActorRef`] which is allowed
/// to be send across thread bounds.
///
/// As with all actor references it can be used to send messages to the actor.
/// To share this reference simply clone it. For more see [`ActorRef`].
///
/// [`ActorSystem`]: ../system/struct.ActorSystem.html
/// [upgraded]: #method.upgrade
/// [`MachineLocalActorRef`]: struct.MachineLocalActorRef.html
/// [`ActorRef`]: enum.ActorRef.html
///
/// # Examples
///
/// ```
/// #![feature(async_await, await_macro, futures_api)]
///
/// use heph::actor::{ActorContext, actor_factory};
/// use heph::system::{ActorSystemBuilder, ActorOptions};
///
/// /// Our actor.
/// async fn actor(mut ctx: ActorContext<String>, _: ()) -> Result<(), ()> {
///     let msg1 = await!(ctx.receive());
///     println!("got first message: {}", msg1);
///     let msg2 = await!(ctx.receive());
///
///     println!("got second message: {}", msg2);
///     Ok(())
/// }
///
/// // Add the actor to the system.
/// let mut actor_system = ActorSystemBuilder::default().build().unwrap();
/// let new_actor = actor_factory(actor);
/// let mut actor_ref = actor_system.add_actor(new_actor, (), ActorOptions::default());
///
/// // Now we can use the reference to send the actor a message, without
/// // having to use `Message` we can just use `String`.
/// actor_ref.send("Hello world".to_owned());
///
/// // To create another `ActorRef` we can simply clone the first one.
/// let mut second_actor_ref = actor_ref.clone();
/// // A now we use that one to send messages as well.
/// second_actor_ref.send("Byte world".to_owned());
/// #
/// # actor_system.run().unwrap();
/// ```
pub struct LocalActorRef<M> {
    /// The inbox of the `Actor`, owned by the `ActorProcess`.
    inbox: WeakShared<MailBox<M>>,
}

impl<M> LocalActorRef<M> {
    /// Create a new `ActorRef` with a shared mailbox.
    pub(crate) const fn new(inbox: WeakShared<MailBox<M>>) -> LocalActorRef<M> {
        LocalActorRef {
            inbox,
        }
    }

    /// Send a message to the actor.
    pub fn send<Msg>(&mut self, msg: Msg) -> Result<(), SendError<Msg>>
        where Msg: Into<M>,
    {
        match self.inbox.upgrade() {
            Some(mut inbox) => inbox.borrow_mut().deliver(msg),
            None => Err(SendError {
                message: msg,
                reason: ErrorReason::ActorShutdown,
            }),
        }
    }

    /// Upgrade the local actor reference to a machine local reference.
    ///
    /// This allows the actor reference to be send across threads, however
    /// operations on it are more expensive.
    pub fn upgrade(self) -> MachineLocalActorRef<M> {
        unimplemented!("LocalActorRef.upgrade");
    }
}

impl<M> Clone for LocalActorRef<M> {
    fn clone(&self) -> LocalActorRef<M> {
        LocalActorRef {
            inbox: self.inbox.clone(),
        }
    }
}

impl<M> fmt::Debug for LocalActorRef<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("LocalActorRef")
            .finish()
    }
}
