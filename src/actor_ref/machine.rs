//! Module containing the machine local actor reference.

use std::fmt;
use std::task::Waker;

use crossbeam_channel::Sender;

use crate::actor_ref::{ActorRef, ActorRefType, SendError};

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
#[allow(missing_debug_implementations)]
pub enum Machine { }

impl<M> ActorRefType<M> for Machine {
    type Data = MachineData<M>;

    fn send(data: &mut Self::Data, msg: M) -> Result<(), SendError<M>> {
        match data.sender.try_send(msg) {
            Ok(()) => {
                data.waker.wake();
                Ok(())
            },
            Err(err) => Err(SendError { message: err.into_inner() }),
        }
    }
}

/// Data used by a machine local actor reference.
pub struct MachineData<M> {
    /// Sending side of the channel to messages to.
    sender: Sender<M>,
    /// A way to notify the actor of the new message.
    waker: Waker,
}

impl<M> Clone for MachineData<M> {
    fn clone(&self) -> MachineData<M> {
        MachineData {
            sender: self.sender.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<M> fmt::Debug for MachineData<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MachineLocalActorRef")
    }
}

impl<M> ActorRef<M, Machine> {
    /// Create a new machine local actor reference.
    ///
    /// The `Waker` must wake the same actor the `Sender` is sending to.
    pub(crate) fn new_machine(sender: Sender<M>, waker: Waker) -> ActorRef<M, Machine> {
        ActorRef::new(MachineData { sender, waker })
    }
}
