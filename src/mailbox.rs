//! Module containing the `MailBox` for an actor.

use std::collections::VecDeque;

use crossbeam_channel::{self as channel, Receiver, Sender};

use crate::scheduler::ProcessId;
use crate::system::ActorSystemRef;

/// Mailbox that holds all messages for an actor.
#[derive(Debug)]
pub struct MailBox<M> {
    /// Process id of the actor, used to notify the actor when receiving
    /// message.
    pid: ProcessId,
    /// Reference to the actor system, used to notify the actor.
    system_ref: ActorSystemRef,
    /// The messages in the mailbox.
    messages: VecDeque<M>,
    /// This is an alternative source of messages, send across thread bounds,
    /// used by machine local actor references to send messages. This defaults
    /// to `None` and is only set to `Some` the first time `upgrade_ref` is
    /// called.
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

    /// Receive the next message, if any.
    pub fn receive_next(&mut self) -> Option<M> {
        self.receive_remote_messages();
        self.messages.pop_front()
    }

    /// Receive all remote messages, if any, and add them to the message queue.
    ///
    /// This is done in an attempt to make the receiving of the two sources of
    /// message a bit more fair.
    fn receive_remote_messages(&mut self) {
        if let Some((_, recv)) = self.messages2.as_ref() {
            loop {
                if let Ok(msg) = recv.try_recv() {
                    self.messages.push_back(msg);
                } else {
                    return;
                }
            }
        }
    }

    /// Used by local actor reference to upgrade to a machine local actor
    /// reference.
    pub fn upgrade_ref(&mut self) -> (ProcessId, Sender<M>) {
        (self.pid, self.messages2.get_or_insert_with(channel::unbounded).0.clone())
    }
}
