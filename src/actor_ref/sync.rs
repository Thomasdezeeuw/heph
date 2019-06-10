//! Module containing the threaded actor reference.

use std::fmt;

use crossbeam_channel::Sender;

use crate::actor_ref::{ActorRef, ActorRefType, SendError};

/// Reference to a synchronous actor.
///
/// A reference to an actor that run in its own thread.
#[allow(missing_debug_implementations)]
pub enum Sync { }

impl<M> ActorRefType<M> for Sync {
    type Data = SyncData<M>;

    fn send(data: &mut Self::Data, msg: M) -> Result<(), SendError<M>> {
        data.sender.try_send(msg)
            .map_err(|err| SendError { message: err.into_inner() })
    }
}

/// Data used [`Sync`].
pub struct SyncData<M> {
    /// Sending side of the channel to messages to.
    sender: Sender<M>,
}

impl<M> Clone for SyncData<M> {
    fn clone(&self) -> SyncData<M> {
        SyncData {
            sender: self.sender.clone(),
        }
    }
}

impl<M> fmt::Debug for SyncData<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SyncActorRef")
    }
}

impl<M> ActorRef<M, Sync> {
    /// Create a new threaded actor reference.
    pub(crate) fn new_sync(sender: Sender<M>) -> ActorRef<M, Sync> {
        ActorRef::new(SyncData { sender })
    }
}
