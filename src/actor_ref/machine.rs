//! Module containing the machine local actor reference.

use std::fmt;
use std::task::Waker;

use crossbeam_channel::Sender;

use crate::actor_ref::{ActorRef, Send, SendError};

/// Machine local actor reference.
///
/// A reference to an actor that can send messages across thread bounds.
///
/// # Notes
///
/// This reference uses much more expensive operations then the local actor
/// reference, **if at all possible prefer to use** [local actor reference]s.
///
/// [local actor reference]: crate::actor_ref::Local
pub struct Machine<M> {
    /// Sending side of the channel to messages to.
    sender: Sender<M>,
    /// A way to notify the actor of the new message.
    waker: Waker,
}

impl<M> Send for Machine<M> {
    type Message = M;

    fn send(&mut self, msg: Self::Message) -> Result<(), SendError<Self::Message>> {
        match self.sender.try_send(msg) {
            Ok(()) => {
                self.waker.wake_by_ref();
                Ok(())
            },
            Err(err) => Err(SendError { message: err.into_inner() }),
        }
    }
}

impl<M> Clone for Machine<M> {
    fn clone(&self) -> Machine<M> {
        Machine {
            sender: self.sender.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<M> fmt::Debug for Machine<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MachineLocalActorRef")
    }
}

impl<M> ActorRef<Machine<M>> {
    /// Create a new machine local actor reference.
    ///
    /// The `Waker` must wake the same actor the `Sender` is sending to.
    pub(crate) fn new_machine(sender: Sender<M>, waker: Waker) -> ActorRef<Machine<M>> {
        ActorRef::new(Machine { sender, waker })
    }
}
