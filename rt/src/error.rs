//! Module containing runtime error types.

use std::any::Any;
use std::{fmt, io};

use crate::{coordinator, worker};

/// Error returned by running a [`Runtime`].
///
/// [`Runtime`]: crate::Runtime
pub struct Error {
    inner: ErrorInner,
}

/// Inside of `Error` error.
enum ErrorInner {
    /// User setup error, created via [`Error::setup`].
    Setup(StringError),
    /// Error setting up tracing infrastructure.
    SetupTrace(io::Error),

    /// Error initialising coordinator.
    InitCoordinator(io::Error),
    /// Error in coordinator.
    Coordinator(coordinator::Error),

    /// Error starting worker thread.
    StartWorker(io::Error),
    /// Error in a worker thread.
    Worker(worker::Error),
    /// Panic in a worker thread.
    WorkerPanic(StringError),

    /// Error starting synchronous actor thread.
    StartSyncActor(io::Error),
    /// Panic in a synchronous actor thread.
    SyncActorPanic(StringError),
}

impl Error {
    /// Create an error to act as user-defined setup error.
    ///
    /// The `err` will be converted into a [`String`] (using [`ToString`], which
    /// is implemented for all types that implement [`fmt::Display`]).
    #[allow(clippy::needless_pass_by_value)]
    pub fn setup<E>(err: E) -> Error
    where
        E: ToString,
    {
        Error {
            inner: ErrorInner::Setup(StringError(err.to_string())),
        }
    }

    pub(crate) const fn setup_trace(err: io::Error) -> Error {
        Error {
            inner: ErrorInner::SetupTrace(err),
        }
    }

    pub(crate) const fn init_coordinator(err: io::Error) -> Error {
        Error {
            inner: ErrorInner::InitCoordinator(err),
        }
    }

    pub(crate) const fn coordinator(err: coordinator::Error) -> Error {
        Error {
            inner: ErrorInner::Coordinator(err),
        }
    }

    pub(crate) const fn start_worker(err: io::Error) -> Error {
        Error {
            inner: ErrorInner::StartWorker(err),
        }
    }

    pub(crate) const fn worker(err: worker::Error) -> Error {
        Error {
            inner: ErrorInner::Worker(err),
        }
    }

    pub(crate) fn worker_panic(err: Box<dyn Any + Send + 'static>) -> Error {
        Error {
            inner: ErrorInner::WorkerPanic(convert_panic(err)),
        }
    }

    pub(crate) const fn start_sync_actor(err: io::Error) -> Error {
        Error {
            inner: ErrorInner::StartSyncActor(err),
        }
    }

    pub(crate) fn sync_actor_panic(err: Box<dyn Any + Send + 'static>) -> Error {
        Error {
            inner: ErrorInner::SyncActorPanic(convert_panic(err)),
        }
    }
}

/// Maps a boxed panic messages to a [`StringError`]
fn convert_panic(err: Box<dyn Any + Send + 'static>) -> StringError {
    let msg = match err.downcast::<&'static str>() {
        Ok(s) => (*s).to_owned(),
        Err(err) => match err.downcast::<String>() {
            Ok(s) => *s,
            Err(..) => "<unknown>".to_owned(),
        },
    };
    StringError(msg)
}

/// We implement [`Debug`] by using [`Display`] implementation because the
/// [`Termination`] trait uses `Debug` rather then `Display` when returning an
/// `Result`.
///
/// [`Termination`]: std::process::Termination
/// [`Debug`]: std::fmt::Debug
/// [`Display`]: std::fmt::Display
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const DESC: &str = "error running Heph runtime";
        match self.inner {
            ErrorInner::Setup(ref err) => {
                write!(f, "{DESC}: error in user-defined setup: {err}")
            }
            ErrorInner::SetupTrace(ref err) => {
                write!(f, "{DESC}: error setting up trace infrastructure: {err}")
            }
            ErrorInner::InitCoordinator(ref err) => {
                write!(f, "{DESC}: error creating coordinator: {err}")
            }
            ErrorInner::Coordinator(ref err) => {
                write!(f, "{DESC}: error in coordinator thread: {err}")
            }
            ErrorInner::StartWorker(ref err) => {
                write!(f, "{DESC}: error starting worker thread: {err}")
            }
            ErrorInner::Worker(ref err) => write!(f, "{DESC}: error in worker thread: {err}"),
            ErrorInner::WorkerPanic(ref err) => write!(f, "{DESC}: panic in worker thread: {err}"),
            ErrorInner::StartSyncActor(ref err) => {
                write!(f, "{DESC}: error starting synchronous actor: {err}")
            }
            ErrorInner::SyncActorPanic(ref err) => {
                write!(f, "{DESC}: panic in synchronous actor thread: {err}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self.inner {
            // All `io::Error`.
            ErrorInner::SetupTrace(ref err)
            | ErrorInner::InitCoordinator(ref err)
            | ErrorInner::StartWorker(ref err)
            | ErrorInner::StartSyncActor(ref err) => Some(err),
            ErrorInner::Coordinator(ref err) => Some(err),
            ErrorInner::Worker(ref err) => Some(err),
            // All `StringError`.
            ErrorInner::Setup(ref err)
            | ErrorInner::WorkerPanic(ref err)
            | ErrorInner::SyncActorPanic(ref err) => Some(err),
        }
    }
}

/// Wrapper around `String` to implement the [`Error`] trait.
///
/// [`Error`]: std::error::Error
pub(crate) struct StringError(String);

impl From<String> for StringError {
    fn from(err: String) -> StringError {
        StringError(err)
    }
}

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
