//! Module containing all errors types.

use std::{fmt, io};
use std::error::Error;

pub(crate) const ERR_SYSTEM_SHUTDOWN: &str = "actor system shutdown";

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
/// use actor::error::{AddActorError, AddActorErrorReason};
///
/// let error = AddActorError {
///     // Actor will be ignored in printing the error.
///     new_actor: (),
///     reason: AddActorErrorReason::SystemShutdown,
/// };
///
/// assert_eq!(error.to_string(), "unable to add actor: actor system shutdown");
/// ```
#[derive(Debug)]
pub struct AddActorError<N> {
    /// The `NewActor` that failed to be added to the system.
    pub new_actor: N,
    /// The reason why the adding failed.
    pub reason: AddActorErrorReason,
}

impl<N> AddActorError<N> {
    /// Description for the error.
    const DESC: &'static str = "unable to add actor";

    /// Create a new `AddActorError`.
    pub(crate) const fn new(new_actor: N, reason: AddActorErrorReason) -> AddActorError<N> {
        AddActorError {
            new_actor,
            reason,
        }
    }
}

impl<N> Into<io::Error> for AddActorError<N> {
    fn into(self) -> io::Error {
        use self::AddActorErrorReason::*;
        match self.reason {
            SystemShutdown => io::Error::new(io::ErrorKind::Other, ERR_SYSTEM_SHUTDOWN),
        }
    }
}

impl<N> fmt::Display for AddActorError<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", AddActorError::<()>::DESC, &self.reason)
    }
}

impl<N: fmt::Debug> Error for AddActorError<N> {
    fn description(&self) -> &str {
        AddActorError::<()>::DESC
    }
}

/// The reason why adding an actor failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum AddActorErrorReason {
    /// The system is shutdown.
    SystemShutdown,
}

impl fmt::Display for AddActorErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::AddActorErrorReason::*;
        match self {
            SystemShutdown => f.pad(ERR_SYSTEM_SHUTDOWN),
        }
    }
}

/// Error when adding initiators to the `ActorSystem`.
///
/// # Notes
///
/// When printing this error (using the `Display` implementation) the initator
/// will not be printed.
///
/// # Examples
///
/// Printing the error doesn't print the actor.
///
/// ```
/// use std::io;
///
/// use actor::error::{AddInitiatorError, AddInitiatorErrorReason};
///
/// let error = AddInitiatorError {
///     // Initiator will be ignored in printing the error.
///     initiator: (),
///     reason: AddInitiatorErrorReason::InitFailed(io::ErrorKind::PermissionDenied.into()),
/// };
///
/// assert_eq!(error.to_string(), "unable to add initiator: permission denied");
/// ```
#[derive(Debug)]
pub struct AddInitiatorError<I> {
    /// The initiator that failed to be added to the system.
    pub initiator: I,
    /// The reason why the adding failed.
    pub reason: AddInitiatorErrorReason,
}

impl<A> AddInitiatorError<A> {
    /// Description for the error.
    const DESC: &'static str = "unable to add initiator";
}

impl<A> Into<io::Error> for AddInitiatorError<A> {
    fn into(self) -> io::Error {
        use self::AddInitiatorErrorReason ::*;
        match self.reason {
            InitFailed(err) => err,
        }
    }
}

impl<A> fmt::Display for AddInitiatorError<A> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", AddInitiatorError::<()>::DESC, &self.reason)
    }
}

impl<A: fmt::Debug> Error for AddInitiatorError<A> {
    fn description(&self) -> &str {
        AddInitiatorError::<()>::DESC
    }

    fn cause(&self) -> Option<&dyn Error> {
        match self.reason {
            AddInitiatorErrorReason::InitFailed(ref err) => Some(err),
        }
    }
}

/// The reason why adding an initiator failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum AddInitiatorErrorReason {
    /// The initialisation of the initiator failed.
    InitFailed(io::Error),
}

impl fmt::Display for AddInitiatorErrorReason  {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::AddInitiatorErrorReason ::*;
        match self {
            InitFailed(ref err) => err.fmt(f),
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
/// use actor::error::{SendError, SendErrorReason};
///
/// let error = SendError {
///     // Message will be ignored in printing the error.
///     message: (),
///     reason: SendErrorReason::ActorShutdown,
/// };
///
/// assert_eq!(error.to_string(), "unable to send message: actor shutdown");
/// ```
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
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
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SendErrorReason {
    /// The actor, to which the message was meant to be sent, is shutdown.
    ActorShutdown,
    /// The system is shutdown.
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
#[non_exhaustive]
pub enum RuntimeError {
    /// Error polling the system poller.
    Poll(io::Error),
}

impl RuntimeError {
    /// Description for the error.
    const DESC: &'static str = "error running actor system";
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::RuntimeError::*;
        match self {
            Poll(ref err) => write!(f, "{}: {}", RuntimeError::DESC, err),
        }
    }
}

impl Error for RuntimeError {
    fn description(&self) -> &str {
        RuntimeError::DESC
    }

    fn cause(&self) -> Option<&dyn Error> {
        use self::RuntimeError::*;
        match self {
            Poll(ref err) => Some(err),
        }
    }
}
