//! Module containing the errors for the `ActorSystem`.

use std::{fmt, io};
use std::error::Error;

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

    fn desc() -> &'static str {
        "unable to add actor"
    }
}

impl<A> fmt::Display for AddActorError<A> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad(AddActorError::<()>::desc())
    }
}

impl<A: fmt::Debug> Error for AddActorError<A> {
    fn description(&self) -> &str {
        AddActorError::<()>::desc()
    }

    fn cause(&self) -> Option<&Error> {
        match self.reason {
            AddActorErrorReason::RegisterFailed(ref err) => Some(err),
            _ => None,
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

impl<M> SendError<M> {
    fn desc() -> &'static str {
        "unable to send message"
    }
}

impl<M> fmt::Display for SendError<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", SendError::<()>::desc(), self.reason)
    }
}

impl<M: fmt::Debug> Error for SendError<M> {
    fn description(&self) -> &str {
        SendError::<()>::desc()
    }
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

impl fmt::Display for SendErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SendErrorReason::ActorShutdown => f.pad("actor is shutdown"),
            SendErrorReason::SystemShutdown=> f.pad("system is shutdown"),
        }
    }
}

/// Error returned by running an `ActorSystem`.
#[derive(Debug)]
pub struct RuntimeError {
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad(self.description())
    }
}

impl Error for RuntimeError {
    fn description(&self) -> &str {
        "error running actor system"
    }
}
