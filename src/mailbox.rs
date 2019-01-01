//! Module containing the `Mailbox` for an actor.

use std::collections::VecDeque;

use crossbeam_channel::{self as channel, Receiver, Sender};

use crate::scheduler::ProcessId;
use crate::system::ActorSystemRef;

/// Mailbox that holds all messages for an actor.
#[derive(Debug)]
pub struct MailBox<M> {
    /// Process id of the actor.
    pid: ProcessId,
    /// Reference to the actor system, used to notify the actor.
    system_ref: ActorSystemRef,
    /// The messages in the mailbox.
    messages: VecDeque<M>,
    /// This is an alternative source of messages, send across thread bounds,
    /// used by `MachineLocalActorRef`s to send messages. This defaults to
    /// `None` and is only set to `Some` if `upgrade_ref` is called.
    messages2: Option<(Sender<M>, Receiver<M>)>,
}

impl<M> MailBox<M> {
    /// Create a new mailbox.
    pub fn new(pid: ProcessId, system_ref: ActorSystemRef) -> MailBox<M> {
        MailBox {
            pid,
            system_ref,
            messages: VecDeque::new(),
            messages2: None,
        }
    }

    /// Deliver a new message to the mailbox.
    ///
    /// This will also schedule the actor to run.
    pub fn deliver(&mut self, msg: M) {
        self.system_ref.notify(self.pid);
        self.messages.push_back(msg);
    }

    /// Receive a delivered message, if any.
    pub fn receive(&mut self) -> Option<M> {
        self.messages2.as_ref()
            .and_then(|(_, recv)| recv.try_recv())
            .or_else(|| self.messages.pop_front())
    }

    /// Used by `LocalActorRef` to upgrade to `MachineLocalActorRef`.
    pub fn upgrade_ref(&mut self) -> (ProcessId, Sender<M>) {
        (self.pid, self.messages2.get_or_insert_with(channel::unbounded).0.clone())
    }
}
