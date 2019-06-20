//! Module containing the local actor reference.

use std::fmt;

use crate::actor_ref::{ActorRef, Send, SendError};
use crate::mailbox::MailBox;
use crate::util::WeakShared;

#[cfg(test)]
use crate::util::Shared;

/// Local actor reference.
///
/// This is a reference to an actor running on the same thread as this reference
/// is located on. This type does not implement `Send` or `Sync`, if this is
/// needed this reference can be [upgraded] to a [machine local actor reference]
/// which is allowed to be send across thread bounds.
///
/// [`ActorSystem`]: crate::system::ActorSystem
/// [upgraded]: crate::actor_ref::ActorRef::upgrade
/// [machine local actor reference]: crate::actor_ref::Machine
pub struct Local<M> {
    /// The inbox of the `Actor`, owned by the `ActorProcess`.
    ///
    /// Note: if this representation changes it will break the Actor Registry!
    pub(super) inbox: WeakShared<MailBox<M>>,
}

impl<M> Send for Local<M> {
    type Message = M;

    fn send(&mut self, msg: Self::Message) -> Result<(), SendError<Self::Message>> {
        match self.inbox.upgrade() {
            Some(mut inbox) => {
                inbox.borrow_mut().deliver(msg);
                Ok(())
            }
            None => Err(SendError { message: msg }),
        }
    }
}

/// Data used by a local actor reference.

impl<M> Clone for Local<M> {
    fn clone(&self) -> Local<M> {
        Local {
            inbox: self.inbox.clone(),
        }
    }
}

impl<M> Eq for Local<M> {}

impl<M> PartialEq for Local<M> {
    fn eq(&self, other: &Local<M>) -> bool {
        self.inbox.ptr_eq(&other.inbox)
    }
}

impl<M> fmt::Debug for Local<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalActorRef")
    }
}

impl<M> ActorRef<Local<M>> {
    /// Create a new `ActorRef` with a shared mailbox.
    pub(crate) const fn new_local(inbox: WeakShared<MailBox<M>>) -> ActorRef<Local<M>> {
        ActorRef::new(Local { inbox })
    }

    /// Get access to the internal inbox, used in testing.
    #[cfg(test)]
    pub(crate) fn get_inbox(&mut self) -> Option<Shared<MailBox<M>>> {
        self.data.inbox.upgrade()
    }
}
