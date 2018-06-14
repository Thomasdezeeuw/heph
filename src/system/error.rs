//! Module containing the errors for the `ActorSystem`.

use std::io;

// TODO: impl Error for the errors below.

/// Error when adding actors to the `ActorSystem`.
#[derive(Debug)]
pub struct AddActorError<A> {
    /// The actor that failed to be added to the system.
    pub actor: A,
    /// The reason why the adding failed.
    pub reason: AddActorErrorReason,
}

impl<A> AddActorError<A> {
    /// Create a new `AddActorError`.
    pub(super) fn new(actor: A, reason: AddActorErrorReason) -> AddActorError<A> {
        AddActorError {
            actor,
            reason,
        }
    }
}

/// The reason why adding an actor failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum AddActorErrorReason {
    /// The system is shutting down.
    SystemShutdown,
    /// The actor failed to be registered with the system poller.
    RegisterFailed(io::Error),
}

/// Error when sending messages goes wrong.
#[derive(Debug, Eq, PartialEq)]
pub struct SendError<M> {
    /// The message that failed to send.
    pub message: M,
    /// The reason why the sending failed.
    pub reason: SendErrorReason,
}

/// The reason why sending a message failed.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum SendErrorReason {
    /// The actor, to which the message was meant to be sent, is shutdown.
    ActorShutdown,

    /// The system is shutting down.
    SystemShutdown,
}

/// Error returned by running an `ActorSystem`.
#[derive(Debug)]
pub struct RuntimeError {
}
