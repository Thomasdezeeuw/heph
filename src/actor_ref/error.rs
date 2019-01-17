//! Module containing actor reference error types.

use std::error::Error;
use std::{fmt, io};

/// Error returned when sending a message.
///
/// This is essentially the same error as [`ActorShutdown`], but allows the
/// message to be retrieved.
///
/// [`ActorShutdown`]: struct.ActorShutdown.html
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
/// use heph::actor_ref::SendError;
///
/// let error = SendError {
///     // Message will be ignored in printing the error.
///     message: (),
/// };
///
/// assert_eq!(error.to_string(), "unable to send message: actor shutdown");
/// ```
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SendError<M> {
    /// The message that failed to send.
    pub message: M,
}

impl<M: fmt::Debug> From<SendError<M>> for io::Error {
    fn from(err: SendError<M>) -> io::Error {
        io::Error::new(io::ErrorKind::Other, err.description())
    }
}

impl<M: fmt::Debug> fmt::Display for SendError<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.description(), ActorShutdown.description())
    }
}

impl<M: fmt::Debug> Error for SendError<M> {
    fn description(&self) -> &str {
        "unable to send message"
    }

    fn cause(&self) -> Option<&dyn Error> {
        Some(&ActorShutdown)
    }
}

/// Error returned when the actor is shutdown.
///
/// This is only possible to detect on local references.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ActorShutdown;

impl From<ActorShutdown> for io::Error {
    fn from(err: ActorShutdown) -> io::Error {
        io::Error::new(io::ErrorKind::Other, err.description())
    }
}

impl fmt::Display for ActorShutdown {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad(self.description())
    }
}

impl Error for ActorShutdown {
    fn description(&self) -> &str {
        "actor shutdown"
    }
}
