//! Module containing system error types.

use std::error::Error;
use std::{fmt, io};

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
