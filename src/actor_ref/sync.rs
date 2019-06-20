//! Module containing the threaded actor reference.

use std::fmt;

use crossbeam_channel::Sender;

use crate::actor_ref::{ActorRef, Send, SendError};

/// Reference to a synchronous actor.
///
/// A reference to an actor that run in its own thread.
pub struct Sync<M> {
    /// Sending side of the channel to messages to.
    sender: Sender<M>,
}

impl<M> Send for Sync<M> {
    type Message = M;

    fn send(&mut self, msg: Self::Message) -> Result<(), SendError<Self::Message>> {
        self.sender.try_send(msg).map_err(|err| SendError {
            message: err.into_inner(),
        })
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
        write!(f, "SyncActorRef")
    }
}

impl<M> ActorRef<Sync<M>> {
    /// Create a new threaded actor reference.
    pub(crate) fn new_sync(sender: Sender<M>) -> ActorRef<Sync<M>> {
        ActorRef::new(Sync { sender })
    }
}
