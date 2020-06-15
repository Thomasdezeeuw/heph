//! Module containing runtime error types.

use std::any::Any;
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
    StartWorker(io::Error),
    /// Error starting synchronous actor thread.
    StartSyncActor(io::Error),
    /// Error in coordinator.
    Coordinator(io::Error),
    /// Error in a worker thread.
    Worker(io::Error),
    /// Error returned by user defined setup function.
    Setup(SetupError),
    /// Panic in a worker thread.
    WorkerPanic(StringError),
    /// Panic in a synchronous actor thread.
    SyncActorPanic(StringError),
}

impl Error<!> {
    /// Helper method to map `Error<!>` to `Error<SetupError>`.
    //
    // TODO: replace this with `From<Error<!>> for
    // Error<SetupError>` impl.
    pub fn map_type<SetupError>(self) -> Error<SetupError> {
        Error {
            inner: match self.inner {
                ErrorInner::StartWorker(err) => ErrorInner::StartWorker(err),
                ErrorInner::StartSyncActor(err) => ErrorInner::StartSyncActor(err),
                ErrorInner::Coordinator(err) => ErrorInner::Coordinator(err),
                ErrorInner::Worker(err) => ErrorInner::Worker(err),
                ErrorInner::<!>::Setup(_) => unreachable!(),
                ErrorInner::WorkerPanic(err) => ErrorInner::WorkerPanic(err),
                ErrorInner::SyncActorPanic(err) => ErrorInner::SyncActorPanic(err),
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

    pub(crate) const fn start_worker(err: io::Error) -> Error<SetupError> {
        Error {
            inner: ErrorInner::StartWorker(err),
        }
    }

    pub(crate) const fn start_sync_actor(err: io::Error) -> Error<SetupError> {
        Error {
            inner: ErrorInner::StartSyncActor(err),
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

    pub(crate) fn worker_panic(err: Box<dyn Any + Send + 'static>) -> Error<SetupError> {
        let msg = convert_panic(err);
        Error {
            inner: ErrorInner::WorkerPanic(msg),
        }
    }

    pub(crate) fn sync_actor_panic(err: Box<dyn Any + Send + 'static>) -> Error<SetupError> {
        let msg = convert_panic(err);
        Error {
            inner: ErrorInner::SyncActorPanic(msg),
        }
    }
}

/// Maps a boxed panic messages to a [`StringError`]
fn convert_panic(err: Box<dyn Any + Send + 'static>) -> StringError {
    let msg = match err.downcast_ref::<&'static str>() {
        Some(s) => (*s).to_owned(),
        None => match err.downcast_ref::<String>() {
            Some(s) => s.clone(),
            None => {
                "unknown panic message (use `String` or `&'static str` in the future)".to_owned()
            }
        },
    };
    StringError(msg)
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
            StartWorker(ref err) => {
                write!(f, "{}: error starting worker thread: {}", Self::DESC, err)
            }
            StartSyncActor(ref err) => write!(
                f,
                "{}: error starting synchronous actor: {}",
                Self::DESC,
                err
            ),
            Coordinator(ref err) => {
                write!(f, "{}: error in coordinator thread: {}", Self::DESC, err)
            }
            Worker(ref err) => write!(f, "{}: error in worker thread: {}", Self::DESC, err),
            Setup(ref err) => write!(f, "{}: error running setup function: {}", Self::DESC, err),
            WorkerPanic(ref msg) => write!(f, "{}: panic in worker thread: {}", Self::DESC, msg),
            SyncActorPanic(ref msg) => write!(
                f,
                "{}: panic in synchronous actor thread: {}",
                Self::DESC,
                msg
            ),
        }
    }
}

impl<SetupError: std::error::Error + 'static> std::error::Error for Error<SetupError> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use ErrorInner::*;
        match self.inner {
            StartWorker(ref err)
            | StartSyncActor(ref err)
            | Coordinator(ref err)
            | Worker(ref err) => Some(err),
            Setup(ref err) => Some(err),
            WorkerPanic(ref err) | SyncActorPanic(ref err) => Some(err),
        }
    }
}

/// Wrapper around `String` to implement the [`Error`] trait.
///
/// [`Error`]: std::error::Error
struct StringError(String);

impl fmt::Debug for StringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for StringError {}
