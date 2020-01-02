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
enum RuntimeErrorInner<SetupError> {
    /// Error starting worker thread.
    StartThread(io::Error),
    /// Error in coordinator.
    Coordinator(io::Error),
    /// Error in a worker thread.
    Worker(io::Error),
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
                RuntimeErrorInner::Coordinator(err) => RuntimeErrorInner::Coordinator(err),
                RuntimeErrorInner::Worker(err) => RuntimeErrorInner::Worker(err),
                RuntimeErrorInner::<!>::Setup(_) => unreachable!(),
                RuntimeErrorInner::Panic(err) => RuntimeErrorInner::Panic(err),
            },
        }
    }
}

impl<SetupError> RuntimeError<SetupError> {
    const DESC: &'static str = "error running actor system";

    pub(crate) const fn coordinator(err: io::Error) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::Coordinator(err),
        }
    }

    pub(crate) const fn start_thread(err: io::Error) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::StartThread(err),
        }
    }

    pub(crate) const fn worker(err: io::Error) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::Worker(err),
        }
    }

    pub(crate) const fn setup(err: SetupError) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::Setup(err),
        }
    }

    pub(crate) const fn panic(err: String) -> RuntimeError<SetupError> {
        RuntimeError {
            inner: RuntimeErrorInner::Panic(err),
        }
    }
}

/// Method to create a setup error, for use outside of the setup function. This
/// is useful for example when setting up a [`tcp::Server`] outside the [setup
/// function in `ActorSystem`] and want to use `RuntimeError` as error returned
/// by main. See example 2 for example usage.
///
/// [`tcp::Server`]: crate::net::tcp::Server::setup
/// [setup function in `ActorSystem`]: crate::system::ActorSystem::with_setup
impl<SetupError> From<SetupError> for RuntimeError<SetupError> {
    fn from(err: SetupError) -> RuntimeError<SetupError> {
        RuntimeError::setup(err)
    }
}

/// We implement [`Debug`] by using [`Display`] implementation because the
/// [`Termination`] trait uses `Debug` rather then `Display` when returning an
/// `Result`.
///
/// [`Termination`]: std::process::Termination
/// [`Debug`]: std::fmt::Debug
/// [`Display`]: std::fmt::Display
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
            Coordinator(ref err) => {
                write!(f, "{}: error in coordinator thread: {}", Self::DESC, err)
            }
            Worker(ref err) => write!(f, "{}: error in worker thread: {}", Self::DESC, err),
            Setup(ref err) => write!(f, "{}: error running setup function: {}", Self::DESC, err),
            Panic(ref msg) => write!(f, "{}: panic in worker thread: {}", Self::DESC, msg),
        }
    }
}

impl<SetupError: Error + fmt::Display + 'static> Error for RuntimeError<SetupError> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        use RuntimeErrorInner::*;
        match self.inner {
            StartThread(ref err) | Coordinator(ref err) | Worker(ref err) => Some(err),
            Setup(ref err) => Some(err),
            Panic(_) => None,
        }
    }
}
