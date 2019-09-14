//! Module containing system error types.

use std::error::Error;
use std::{fmt, io};

/// Error returned by running an [`ActorSystem`].
///
/// [`ActorSystem`]: crate::system::ActorSystem
pub struct RuntimeError<SetupError = !> {
    inner: RuntimeErrorInner<SetupError>,
}

/// Inside of `RuntimeError` error.
pub(crate) enum RuntimeErrorInner<SetupError> {
    /// Error starting worker thread.
    StartThread(io::Error),
    /// Error polling the system poller.
    Poll(io::Error),
    /// Error returned by user defined setup function.
    Setup(SetupError),
    /// Panic in a worker thread.
    Panic(String),
}

impl RuntimeError<!> {
    /// Helper method to map `RuntimeError<!>` to `RuntimeError<SetupError>`.
    //
    // TODO: replace this with `From<RuntimeError<!>> for
    // RuntimeError<SetupError>` impl.
    pub fn map_type<SetupError>(self) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: match self.inner {
                RuntimeErrorInner::StartThread(err) => RuntimeErrorInner::StartThread(err),
                RuntimeErrorInner::Poll(err) => RuntimeErrorInner::Poll(err),
                RuntimeErrorInner::<!>::Setup(_) => unreachable!(),
                RuntimeErrorInner::Panic(err) => RuntimeErrorInner::Panic(err),
            },
        }
    }
}

impl<SetupError> RuntimeError<SetupError> {
    const DESC: &'static str = "error running actor system";

    pub(crate) fn start_thread(err: io::Error) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::StartThread(err),
        }
    }

    pub(crate) fn poll(err: io::Error) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::Poll(err),
        }
    }

    /// Method to create a setup error, for use outside of the setup function.
    /// This is useful for example when setting up a [`tcp::Server`] outside the
    /// [setup function in `ActorSystem`] and want to use `RuntimeError` as
    /// error returned by main. See example 2 for example usage.
    ///
    /// [`tcp::Server`]: crate::net::tcp::Server::setup
    /// [setup function in `ActorSystem`]: crate::system::ActorSystem::with_setup
    pub fn setup(err: SetupError) -> RuntimeError<SetupError> {
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

// We implement `Debug` by using `Display` implementation because `Termination`
// uses `Debug` rather then `Display`.
impl<SetupError: fmt::Display> fmt::Debug for RuntimeError<SetupError> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<SetupError: fmt::Display> fmt::Display for RuntimeError<SetupError> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use RuntimeErrorInner::*;
        match self.inner {
            StartThread(ref err) => {
                write!(f, "{}: error starting worker thread: {}", Self::DESC, err)
            }
            Poll(ref err) => write!(f, "{}: error polling system poller: {}", Self::DESC, err),
            Setup(ref err) => write!(f, "{}: error running setup function: {}", Self::DESC, err),
            Panic(ref msg) => write!(f, "{}: panic in worker thread: {}", Self::DESC, msg),
        }
    }
}

impl<SetupError: Error + fmt::Display + 'static> Error for RuntimeError<SetupError> {
    fn description(&self) -> &str {
        Self::DESC
    }

    fn cause(&self) -> Option<&dyn Error> {
        self.source()
    }

    fn source(&self) -> Option<&(dyn Error + 'static)> {
        use RuntimeErrorInner::*;
        match self.inner {
            StartThread(ref err) | Poll(ref err) => Some(err),
            Setup(ref err) => Some(err),
            Panic(_) => None,
        }
    }
}
