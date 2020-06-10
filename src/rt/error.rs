//! Module containing runtime error types.

use std::{fmt, io};

/// Error returned by running an [`Runtime`].
///
/// [`Runtime`]: crate::Runtime
pub struct Error<SetupError = !> {
    inner: ErrorInner<SetupError>,
}

/// Inside of `Error` error.
enum ErrorInner<SetupError> {
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

impl Error<!> {
    /// Helper method to map `Error<!>` to `Error<SetupError>`.
    //
    // TODO: replace this with `From<Error<!>> for
    // Error<SetupError>` impl.
    pub fn map_type<SetupError>(self) -> Error<SetupError> {
        Error {
            inner: match self.inner {
                ErrorInner::StartThread(err) => ErrorInner::StartThread(err),
                ErrorInner::Coordinator(err) => ErrorInner::Coordinator(err),
                ErrorInner::Worker(err) => ErrorInner::Worker(err),
                ErrorInner::<!>::Setup(_) => unreachable!(),
                ErrorInner::Panic(err) => ErrorInner::Panic(err),
            },
        }
    }
}

impl<SetupError> Error<SetupError> {
    const DESC: &'static str = "error running Heph runtime";

    pub(crate) const fn coordinator(err: io::Error) -> Error<SetupError> {
        Error {
            inner: ErrorInner::Coordinator(err),
        }
    }

    pub(crate) const fn start_thread(err: io::Error) -> Error<SetupError> {
        Error {
            inner: ErrorInner::StartThread(err),
        }
    }

    pub(crate) const fn worker(err: io::Error) -> Error<SetupError> {
        Error {
            inner: ErrorInner::Worker(err),
        }
    }

    pub(crate) const fn setup(err: SetupError) -> Error<SetupError> {
        Error {
            inner: ErrorInner::Setup(err),
        }
    }

    pub(crate) const fn panic(err: String) -> Error<SetupError> {
        Error {
            inner: ErrorInner::Panic(err),
        }
    }
}

/// Method to create a setup error, for use outside of the setup function. This
/// is useful for example when setting up a [`tcp::Server`] outside the [setup
/// function in `Runtime`] and want to use `Error` as error returned by
/// main. See example 2 for example usage.
///
/// [`tcp::Server`]: crate::net::tcp::Server::setup
/// [setup function in `Runtime`]: crate::Runtime::with_setup
impl<SetupError> From<SetupError> for Error<SetupError> {
    fn from(err: SetupError) -> Error<SetupError> {
        Error::setup(err)
    }
}

/// We implement [`Debug`] by using [`Display`] implementation because the
/// [`Termination`] trait uses `Debug` rather then `Display` when returning an
/// `Result`.
///
/// [`Termination`]: std::process::Termination
/// [`Debug`]: std::fmt::Debug
/// [`Display`]: std::fmt::Display
impl<SetupError: fmt::Display> fmt::Debug for Error<SetupError> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<SetupError: fmt::Display> fmt::Display for Error<SetupError> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ErrorInner::*;
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

impl<SetupError: std::error::Error + 'static> std::error::Error for Error<SetupError> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use ErrorInner::*;
        match self.inner {
            StartThread(ref err) | Coordinator(ref err) | Worker(ref err) => Some(err),
            Setup(ref err) => Some(err),
            Panic(_) => None,
        }
    }
}
