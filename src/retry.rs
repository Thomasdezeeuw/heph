use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{self, Poll};

use crate::actor::{NoMessages, RecvError};

/// Retry the function `f` with `strategy`.
pub fn retry<S, F, Fut, T, E>(strategy: S, mut f: F) -> Retry<F, Fut, S>
where
    S: Strategy<E>,
    F: FnMut() -> Fut,
    E: IsTemporary,
{
    let fut = f();
    Retry { f, fut, strategy }
}

/// [`Future`] returned by [`retry`].
#[derive(Debug)]
pub struct Retry<F, Fut, S> {
    /// Function to create the future.
    f: F,
    /// Future we're actually running.
    fut: Fut,
    /// [`Strategy`] to determine whether or not to retry.
    strategy: S,
}

impl<F, Fut, S, T, E> Future for Retry<F, Fut, S>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    S: Strategy<E>,
    E: IsTemporary,
{
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { Pin::into_inner_unchecked(self) };
        loop {
            match unsafe { Pin::new_unchecked(&mut this.fut) }.poll(ctx) {
                Poll::Ready(Ok(ok)) => return Poll::Ready(Ok(ok)),
                Poll::Ready(Err(err)) => {
                    if err.is_temporary() {
                        match this.strategy.should_retry(err) {
                            Retry2::Retry => {
                                this.fut = (this.f)();
                                continue;
                            }
                            Retry2::Stop(err) => return Poll::Ready(Err(err)),
                        }
                    } else {
                        return Poll::Ready(Err(err));
                    }
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Retry strategy used to determine whether an operation should be retried or
/// not.
pub trait Strategy<E> {
    /// Determine if an operation should be retried based on the `err`or.
    fn should_retry(&mut self, err: E) -> Retry2<E>;
}

// TODO: better name.
#[derive(Debug)]
pub enum Retry2<E> {
    /// Retry the operation.
    Retry,
    /// Stop retrying.
    Stop(E),
}

/// Determine whether or not an error is temporary.
pub trait IsTemporary {
    /// Returns `true` if the error is temporary and *could* go away when the
    /// operation is simply retried, returns `false` we this will never be the
    /// case.
    fn is_temporary(&self) -> bool;
}

impl IsTemporary for io::Error {
    fn is_temporary(&self) -> bool {
        use io::ErrorKind::*;
        use libc::*;

        match self.raw_os_error() {
            #[allow(unreachable_patterns)] // `EAGAIN` and `EWOULDBLOCK` may have the same value.
            Some(
                // NOTE: we could recover from `ECONNRESET` and `ENETRESET`, but
                // would require a reconnect.
                EAGAIN | EWOULDBLOCK | EINTR | EINPROGRESS | EBUSY | ENETDOWN | ENOLCK | ENOLINK
                | ETIME | ETIMEDOUT,
            ) => true,
            Some(_) => false,
            None => match self.kind() {
                // NOTE: `NetworkDown` might be temporary, it might not, with a
                // decent retry strategy this shouldn't be a problem.
                ExecutableFileBusy | Interrupted | NetworkDown | TimedOut | WouldBlock => true,
                // NOTE: we could recover from `ConnectionAborted` and
                // `ConnectionReset`, but would require a reconnect.
                _ => false,
            },
        }
    }
}

impl IsTemporary for NoMessages {
    fn is_temporary(&self) -> bool {
        false
    }
}

impl IsTemporary for RecvError {
    fn is_temporary(&self) -> bool {
        match self {
            RecvError::Empty => true,
            RecvError::Disconnected => false,
        }
    }
}
