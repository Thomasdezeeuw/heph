//! Module containing the threaded actor reference.

use std::fmt;

use crossbeam_channel::Sender;

use crate::actor_ref::{ActorRef, Send, SendError};

/// Reference to a synchronous actor.
///
/// For more information about synchronous actors see the [`actor::sync`]
/// module.
///
/// [`actor::sync`]: crate::actor::sync
pub struct Sync<M> {
    /// Sending side of the channel to messages to.
    sender: Sender<M>,
}

impl<M> Send for Sync<M> {
    type Message = M;

    fn send(&mut self, msg: Self::Message) -> Result<(), SendError> {
        self.sender.try_send(msg).map_err(|_err| SendError)
    }
}

impl<M> Eq for Sync<M> {}

impl<M> PartialEq for Sync<M> {
    fn eq(&self, other: &Sync<M>) -> bool {
        self.sender.same_channel(&other.sender)
    }
}

impl<M> Clone for Sync<M> {
    fn clone(&self) -> Sync<M> {
        Sync {
            sender: self.sender.clone(),
        }
    }
}

impl<M> fmt::Debug for Sync<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SyncActorRef")
    }
}

impl<M> ActorRef<Sync<M>> {
    /// Create a new threaded actor reference.
    pub(crate) const fn new_sync(sender: Sender<M>) -> ActorRef<Sync<M>> {
        ActorRef::new(Sync { sender })
    }
}
