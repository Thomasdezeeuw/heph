//! Module containing the `Mailbox` for an `ActorProcess`.

use std::collections::VecDeque;

use crossbeam_channel::{self as channel, Receiver, Sender};
use mio_st::event::Ready;
use mio_st::registration::Notifier;

use crate::process::ProcessId;
use crate::error::SendError;

/// Mailbox that holds all messages for an `Actor`.
#[derive(Debug)]
pub struct MailBox<M> {
    /// Process id of the actor.
    pid: ProcessId,
    /// The messages in the mailbox.
    messages: VecDeque<M>,
    /// Actor specific notifier.
    notifier: Notifier,
    /// This is an alternative source of messages, send across thread bounds,
    /// used by `MachineLocalActorRef`s to send messages. This defaults to
    /// `None` and is only set to `Some` if `upgrade_ref` is called.
    messages2: Option<(Sender<M>, Receiver<M>)>,
}

impl<M> MailBox<M> {
    /// Create a new mailbox.
    pub fn new(pid: ProcessId, notifier: Notifier) -> MailBox<M> {
        MailBox {
            pid,
            messages: VecDeque::new(),
            notifier,
            messages2: None,
        }
    }

    /// Deliver a new message to the mailbox.
    ///
    /// This will also schedule the actor to run.
    pub fn deliver<Msg>(&mut self, msg: Msg) -> Result<(), SendError<Msg>>
        where Msg: Into<M>,
    {
        match self.notifier.notify(Ready::READABLE) {
            Ok(()) => {
                self.messages.push_back(msg.into());
                Ok(())
            },
            Err(_) => Err(SendError { message: msg }),
        }
    }

    /// Receive a delivered message, if any.
    pub fn receive(&mut self) -> Option<M> {
        self.messages2.as_mut()
            .and_then(|(_, recv)| recv.try_recv())
            .or_else(|| self.messages.pop_front())
    }

    /// Used by `LocalActorRef` to upgrade to `MachineLocalActorRef`.
    pub(crate) fn upgrade_ref(&mut self) -> (ProcessId, Sender<M>) {
        (self.pid, self.messages2.get_or_insert_with(channel::unbounded).0.clone())
    }
}
