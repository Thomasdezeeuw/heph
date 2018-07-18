//! Module containing the `Mailbox` for an `ActorProcess`.

use std::collections::VecDeque;

use mio_st::event::Ready;
use mio_st::registration::Notifier;

use crate::system::ActorSystemRef;
use crate::system::error::{SendError, SendErrorReason};

/// Mailbox that holds all messages for an `Actor`.
#[derive(Debug)]
pub struct MailBox<M> {
    /// The messages in the mailbox.
    messages: VecDeque<M>,
    /// Actor specific notifier.
    notifier: Notifier,
    /// A reference to the actor system, used to check if it's still running.
    system_ref: ActorSystemRef,
}

impl<M> MailBox<M> {
    /// Create a new mailbox.
    pub fn new(notifier: Notifier, system_ref: ActorSystemRef) -> MailBox<M> {
        MailBox {
            messages: VecDeque::new(),
            notifier,
            system_ref,
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
                if self.system_ref.is_shutdown() {
                    Err(SendError {
                        message: msg,
                        reason: SendErrorReason::SystemShutdown,
                    })
                } else {
                    self.messages.push_back(msg.into());
                    Ok(())
                }
            },
            Err(_) => Err(SendError {
                message: msg,
                reason: SendErrorReason::ActorShutdown,
            }),
        }
    }

    /// Receive a delivered message, if any.
    pub fn receive(&mut self) -> Option<M> {
        self.messages.pop_front()
    }
}
