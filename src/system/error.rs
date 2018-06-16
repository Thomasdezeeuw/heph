//! Module containing the errors for the `ActorSystem`.

use std::{fmt, io};
use std::error::Error;

const ERR_SYSTEM_SHUTDOWN: &str = "actor system shutdown";

/// Error when adding actors to the `ActorSystem`.
///
/// # Notes
///
/// When printing this error (using the `Display` implementation) the actor will
/// not be printed.
///
/// # Examples
///
/// Printing the error doesn't print the actor.
///
/// ```
/// use actor::system::error::{AddActorError, AddActorErrorReason};
///
/// let error = AddActorError {
///     // Actor will be ignored in printing the error.
///     actor: (),
///     reason: AddActorErrorReason::SystemShutdown,
/// };
///
/// assert_eq!(error.to_string(), "unable to add actor: actor system shutdown");
/// ```
#[derive(Debug)]
pub struct AddActorError<A> {
    /// The actor that failed to be added to the system.
    pub actor: A,
    /// The reason why the adding failed.
    pub reason: AddActorErrorReason,
}

impl<A> AddActorError<A> {
    /// Description for the error.
    const DESC: &'static str = "unable to add actor";

    /// Create a new `AddActorError`.
    pub(super) const fn new(actor: A, reason: AddActorErrorReason) -> AddActorError<A> {
        AddActorError {
            actor,
            reason,
        }
    }
}

impl<A> Into<io::Error> for AddActorError<A> {
    fn into(self) -> io::Error {
        use self::AddActorErrorReason::*;
        match self.reason {
            SystemShutdown => io::Error::new(io::ErrorKind::Other, ERR_SYSTEM_SHUTDOWN),
            RegisterFailed(err) => err,
        }
    }
}

impl<A> fmt::Display for AddActorError<A> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", AddActorError::<()>::DESC, &self.reason)
    }
}

impl<A: fmt::Debug> Error for AddActorError<A> {
    fn description(&self) -> &str {
        AddActorError::<()>::DESC
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

impl fmt::Display for AddActorErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::AddActorErrorReason::*;
        match self {
            SystemShutdown => f.pad(ERR_SYSTEM_SHUTDOWN),
            RegisterFailed(ref err) => err.fmt(f),
        }
    }
}

/// Error when sending messages goes wrong.
///
/// # Notes
///
/// When printing this error (using the `Display` implementation) the message
/// will not be printed.
///
/// # Examples
///
/// Printing the error doesn't print the message.
///
/// ```
/// use actor::system::error::{SendError, SendErrorReason};
///
/// let error = SendError {
///     // Message will be ignored in printing the error.
///     message: (),
///     reason: SendErrorReason::ActorShutdown,
/// };
///
/// assert_eq!(error.to_string(), "unable to send message: actor shutdown");
/// ```
#[derive(Debug)]
pub struct SendError<M> {
    /// The message that failed to send.
    pub message: M,
    /// The reason why the sending failed.
    pub reason: SendErrorReason,
}

impl<M> SendError<M> {
    /// Description for the error.
    const DESC: &'static str = "unable to send message";
}

impl<M> fmt::Display for SendError<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", SendError::<()>::DESC, &self.reason)
    }
}

impl<M: fmt::Debug> Error for SendError<M> {
    fn description(&self) -> &str {
        SendError::<()>::DESC
    }
}

/// The reason why sending a message failed.
#[derive(Debug)]
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
            SendErrorReason::ActorShutdown => f.pad("actor shutdown"),
            SendErrorReason::SystemShutdown=> f.pad(ERR_SYSTEM_SHUTDOWN),
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
