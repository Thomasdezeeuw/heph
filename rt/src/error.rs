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
    const DESC: &'static str = "error running Heph runtime";

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

    pub(super) const fn setup_trace(err: io::Error) -> Error {
        Error {
            inner: ErrorInner::SetupTrace(err),
        }
    }

    pub(super) const fn init_coordinator(err: io::Error) -> Error {
        Error {
            inner: ErrorInner::InitCoordinator(err),
        }
    }

    pub(super) const fn coordinator(err: coordinator::Error) -> Error {
        Error {
            inner: ErrorInner::Coordinator(err),
        }
    }

    pub(super) const fn start_worker(err: io::Error) -> Error {
        Error {
            inner: ErrorInner::StartWorker(err),
        }
    }

    pub(super) const fn worker(err: worker::Error) -> Error {
        Error {
            inner: ErrorInner::Worker(err),
        }
    }

    pub(super) fn worker_panic(err: Box<dyn Any + Send + 'static>) -> Error {
        let msg = convert_panic(err);
        Error {
            inner: ErrorInner::WorkerPanic(msg),
        }
    }

    pub(super) const fn start_sync_actor(err: io::Error) -> Error {
        Error {
            inner: ErrorInner::StartSyncActor(err),
        }
    }

    pub(super) fn sync_actor_panic(err: Box<dyn Any + Send + 'static>) -> Error {
        let msg = convert_panic(err);
        Error {
            inner: ErrorInner::SyncActorPanic(msg),
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
        use ErrorInner::*;
        let desc = Self::DESC;
        match self.inner {
            Setup(ref err) => {
                write!(f, "{desc}: error in user-defined setup: {err}")
            }
            SetupTrace(ref err) => {
                write!(f, "{desc}: error setting up trace infrastructure: {err}")
            }
            InitCoordinator(ref err) => {
                write!(f, "{desc}: error creating coordinator: {err}")
            }
            Coordinator(ref err) => {
                write!(f, "{desc}: error in coordinator thread: {err}")
            }
            StartWorker(ref err) => {
                write!(f, "{desc}: error starting worker thread: {err}")
            }
            Worker(ref err) => write!(f, "{desc}: error in worker thread: {err}"),
            WorkerPanic(ref err) => write!(f, "{desc}: panic in worker thread: {err}"),
            StartSyncActor(ref err) => {
                write!(f, "{desc}: error starting synchronous actor: {err}")
            }
            SyncActorPanic(ref err) => {
                write!(f, "{desc}: panic in synchronous actor thread: {err}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        use ErrorInner::*;
        match self.inner {
            // All `io::Error`.
            SetupTrace(ref err)
            | InitCoordinator(ref err)
            | StartWorker(ref err)
            | StartSyncActor(ref err) => Some(err),
            Coordinator(ref err) => Some(err),
            Worker(ref err) => Some(err),
            // All `StringError`.
            Setup(ref err) | WorkerPanic(ref err) | SyncActorPanic(ref err) => Some(err),
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
