//! Module containing all errors types.

use std::error::Error;
use std::{fmt, io};

/// Error returned when the actor is shutdown.
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

/// Error returned by running an [`ActorSystem`].
///
/// [`ActorSystem`]: ../system/struct.ActorSystem.html
#[derive(Debug)]
pub struct RuntimeError {
    inner: RuntimeErrorInner,
}

/// Inside of `RuntimeError` error.
#[derive(Debug)]
pub(crate) enum RuntimeErrorInner {
    /// Error polling the system poller.
    Poll(io::Error),
    /// Error return by initialising an initiator.
    Initiator(io::Error),
    /// Error returned by user defined setup function.
    Setup(io::Error),
    /// Panic in a worker thread.
    Panic(String),
}

impl RuntimeError {
    pub(crate) fn poll(err: io::Error) -> RuntimeError {
        RuntimeError {
            inner: RuntimeErrorInner::Poll(err),
        }
    }

    pub(crate) fn initiator(err: io::Error) -> RuntimeError {
        RuntimeError {
            inner: RuntimeErrorInner::Initiator(err),
        }
    }

    pub(crate) fn setup(err: io::Error) -> RuntimeError {
        RuntimeError {
            inner: RuntimeErrorInner::Setup(err),
        }
    }

    pub(crate) fn panic(err: String) -> RuntimeError {
        RuntimeError {
            inner: RuntimeErrorInner::Panic(err),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::RuntimeErrorInner::*;
        match self.inner {
            Poll(ref err) => write!(f, "{}: error polling system poller: {}",
                self.description(), err),
            Initiator(ref err) => write!(f, "{}: error initialising initiator: {}",
                self.description(), err),
            Setup(ref err) => write!(f, "{}: error running setup function: {}",
                self.description(), err),
            Panic(ref msg) => write!(f, "{}: caught panic worker thread: {}",
                self.description(), msg),
        }
    }
}

impl Error for RuntimeError {
    fn description(&self) -> &str {
        "error running actor system"
    }

    fn cause(&self) -> Option<&dyn Error> {
        use self::RuntimeErrorInner::*;
        match self.inner {
            Poll(ref err) | Initiator(ref err) | Setup(ref err) => Some(err),
            Panic(_) => None,
        }
    }
}

/// Error when sending a message.
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
/// use heph::error::SendError;
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
