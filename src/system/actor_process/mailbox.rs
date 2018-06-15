//! Module containing the `Mailbox`.

use std::collections::VecDeque;

use mio_st::event::Ready;
use mio_st::registration::Notifier;

use system::error::{SendError, SendErrorReason};

/// Mailbox that holds all messages for an `Actor`.
#[derive(Debug)]
pub struct MailBox<M> {
    /// The messages in the mailbox.
    messages: VecDeque<M>,
    /// The notifier for the process.
    notifier: Notifier,
}

impl<M> MailBox<M> {
    /// Create a new mailbox.
    ///
    /// The `Notifier` must be registered with at least `READABLE` interest.
    pub fn new(mut notifier: Notifier) -> MailBox<M> {
        debug_assert!(notifier.interests().unwrap().is_readable(),
            "notifier not registered with `READABLE` interest");

        MailBox {
            messages: VecDeque::new(),
            notifier,
        }
    }

    /// Deliver a new message to the mailbox.
    pub fn deliver(&mut self, msg: M) -> Result<(), SendError<M>> {
        match self.notifier.notify(Ready::READABLE) {
            Ok(()) => {
                self.messages.push_back(msg);
                Ok(())
            },
            Err(_) => {
                // TODO: detect ActorSystem shutdown.
                Err(SendError {
                    message: msg,
                    reason: SendErrorReason::ActorShutdown,
                })
            },
        }
    }

    /// Receive a delivered message, if any.
    pub fn receive(&mut self) -> Option<M> {
        self.messages.pop_front()
    }
}
