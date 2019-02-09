//! Module containing the local actor reference.

use std::fmt;

use crate::actor_ref::{ActorRef, ActorRefType, SendError};
use crate::mailbox::MailBox;
use crate::util::WeakShared;

#[cfg(all(test, feature = "test"))]
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
#[allow(missing_debug_implementations)]
pub enum Local { }

impl<M> ActorRefType<M> for Local {
    type Data = LocalData<M>;

    fn send(data: &mut Self::Data, msg: M) -> Result<(), SendError<M>> {
        match data.inbox.upgrade() {
            Some(mut inbox) => {
                inbox.borrow_mut().deliver(msg);
                Ok(())
            },
            None => Err(SendError { message: msg }),
        }
    }
}

/// Data used by a local actor reference.
pub struct LocalData<M> {
    /// The inbox of the `Actor`, owned by the `ActorProcess`.
    ///
    /// Note: if this representation changes it will break the Actor Registry!
    pub(super) inbox: WeakShared<MailBox<M>>,
}

impl<M> Clone for LocalData<M> {
    fn clone(&self) -> LocalData<M> {
        LocalData {
            inbox: self.inbox.clone(),
        }
    }
}

impl<M> Eq for LocalData<M> {}

impl<M> PartialEq for LocalData<M> {
    fn eq(&self, other: &LocalData<M>) -> bool {
        self.inbox.ptr_eq(&other.inbox)
    }
}

impl<M> fmt::Debug for LocalData<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "LocalActorRef")
    }
}

impl<M> ActorRef<M, Local> {
    /// Create a new `ActorRef` with a shared mailbox.
    pub(crate) const fn new(inbox: WeakShared<MailBox<M>>) -> ActorRef<M, Local> {
        ActorRef {
            data: LocalData { inbox },
        }
    }

    /// Get access to the internal inbox, used in testing.
    #[cfg(all(test, feature = "test"))]
    pub(crate) fn get_inbox(&mut self) -> Option<Shared<MailBox<M>>> {
        self.data.inbox.upgrade()
    }
}
