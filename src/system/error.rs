//! Module containing system error types.

use std::error::Error;
use std::{fmt, io};

/// Error returned by running an [`ActorSystem`].
///
/// [`ActorSystem`]: ../system/struct.ActorSystem.html
#[derive(Debug)]
pub struct RuntimeError<SetupError = !> {
    inner: RuntimeErrorInner<SetupError>,
}

/// Inside of `RuntimeError` error.
#[derive(Debug)]
pub(crate) enum RuntimeErrorInner<SetupError> {
    /// Error polling the system poller.
    Poll(io::Error),
    /// Error returned by user defined setup function.
    Setup(SetupError),
    /// Panic in a worker thread.
    Panic(String),
}

impl<SetupError> RuntimeError<SetupError> {
    const DESC: &'static str = "error running actor system";

    pub(crate) fn poll(err: io::Error) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::Poll(err),
        }
    }

    pub(crate) fn setup(err: SetupError) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::Setup(err),
        }
    }

    pub(crate) fn panic(err: String) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::Panic(err),
        }
    }
}

impl<SetupError: fmt::Display> fmt::Display for RuntimeError<SetupError> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::RuntimeErrorInner::*;
        match self.inner {
            Poll(ref err) => write!(f, "{}: error polling system poller: {}",
                Self::DESC, err),
            Setup(ref err) => write!(f, "{}: error running setup function: {}",
                Self::DESC, err),
            Panic(ref msg) => write!(f, "{}: caught panic worker thread: {}",
                Self::DESC, msg),
        }
    }
}

impl<SetupError: Error + fmt::Display> Error for RuntimeError<SetupError> {
    fn description(&self) -> &str {
        Self::DESC
    }

    fn cause(&self) -> Option<&dyn Error> {
        use self::RuntimeErrorInner::*;
        match self.inner {
            Poll(ref err) => Some(err),
            Setup(ref err) => Some(err),
            Panic(_) => None,
        }
    }
}
