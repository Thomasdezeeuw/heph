//! Module containing actor reference error types.

use std::error::Error;
use std::fmt;

/// Error returned when sending a message.
///
/// # Notes
///
/// When printing this error (using either the `Display` or `Debug`
/// implementation) the message will not be printed.
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
/// assert_eq!(error.to_string(), "unable to send message");
/// ```
#[derive(Copy, Clone, Eq, PartialEq)]
pub struct SendError<M> {
    /// The message that failed to be send.
    pub message: M,
}

impl<M> fmt::Debug for SendError<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("SendError").finish()
    }
}

impl<M> fmt::Display for SendError<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

impl<M> Error for SendError<M> {
    fn description(&self) -> &str {
        "unable to send message"
    }
}

/// Error returned when the actor is shutdown.
///
/// This is only possible to detect on local references.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ActorShutdown;

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
