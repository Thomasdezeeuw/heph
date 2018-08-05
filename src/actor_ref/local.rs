//! Module containing the `LocalActorRef`.

use std::fmt;

use crate::actor_ref::MachineLocalActorRef;
use crate::error::{ActorShutdown, SendError};
use crate::mailbox::MailBox;
use crate::system::ActorSystemRef;
use crate::util::WeakShared;
use crate::waker::new_waker;

/// A reference to a local actor inside a [`ActorSystem`].
///
/// This is a reference to an actor running on the same thread as this reference
/// is on. This type does not implement `Send` or `Sync`, if this is needed this
/// reference can be [upgraded] to a [`MachineLocalActorRef`] which is allowed
/// to be send across thread bounds.
///
/// [`ActorSystem`]: ../system/struct.ActorSystem.html
/// [upgraded]: #method.upgrade
/// [`MachineLocalActorRef`]: struct.MachineLocalActorRef.html
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
            None => Err(SendError { message: msg }),
        }
    }

    /// Upgrade the local actor reference to a machine local reference.
    ///
    /// This allows the actor reference to be send across threads, however
    /// operations on it are more expensive.
    pub fn upgrade(self, system_ref: &mut ActorSystemRef) -> Result<MachineLocalActorRef<M>, ActorShutdown> {
        let (pid, sender) = match self.inbox.upgrade() {
            Some(mut inbox) => inbox.borrow_mut().upgrade_ref(),
            None => return Err(ActorShutdown),
        };

        let notification_sender = system_ref.get_notification_sender();
        let waker = new_waker(pid, notification_sender);
        Ok(MachineLocalActorRef::new(sender, waker.into()))
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
