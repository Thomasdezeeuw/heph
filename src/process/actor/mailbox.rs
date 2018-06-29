//! Module containing the `Mailbox` for an `ActorProcess`.

use std::collections::VecDeque;

use process::ProcessId;
use system::ActorSystemRef;
use system::error::{SendError, SendErrorReason};

/// Mailbox that holds all messages for an `Actor`.
#[derive(Debug)]
pub struct MailBox<M> {
    /// The messages in the mailbox.
    messages: VecDeque<M>,
    /// The process id of the actor process to which this mailbox belongs.
    pid: ProcessId,
    /// A reference to the actor system.
    system_ref: ActorSystemRef,
}

impl<M> MailBox<M> {
    /// Create a new mailbox.
    pub fn new(pid: ProcessId, system_ref: ActorSystemRef) -> MailBox<M> {
        MailBox {
            messages: VecDeque::new(),
            pid,
            system_ref,
        }
    }

    /// Deliver a new message to the mailbox.
    ///
    /// This will schedule the actor to run.
    pub fn deliver<Msg>(&mut self, msg: Msg) -> Result<(), SendError<Msg>>
        where Msg: Into<M>,
    {
        match self.system_ref.schedule(self.pid) {
            Ok(()) => {
                self.messages.push_back(msg.into());
                Ok(())
            },
            Err(()) => Err(SendError {
                message: msg,
                reason: SendErrorReason::SystemShutdown,
            }),
        }
    }

    /// Receive a delivered message, if any.
    pub fn receive(&mut self) -> Option<M> {
        self.messages.pop_front()
    }
}
