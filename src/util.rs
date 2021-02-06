//! Module with various utilities.

use std::future::Future;
use std::pin::Pin;
use std::stream::Stream;
use std::task::{self, Poll};

/// Helper [`Future`] that poll `future1` and `future2` and returns the output
/// of the future that completes first.
pub const fn either<Fut1, Fut2>(future1: Fut1, future2: Fut2) -> Either<Fut1, Fut2> {
    Either { future1, future2 }
}

/// The [`Future`] behind [`either`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Either<Fut1, Fut2> {
    future1: Fut1,
    future2: Fut2,
}

impl<Fut1, Fut2> Future for Either<Fut1, Fut2>
where
    Fut1: Future,
    Fut2: Future,
{
    type Output = Result<Fut1::Output, Fut2::Output>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // Safety: not moving `future1`.
        let future1 = unsafe { Pin::map_unchecked_mut(self.as_mut(), |s| &mut s.future1) };
        match future1.poll(ctx) {
            Poll::Ready(value) => Poll::Ready(Ok(value)),
            Poll::Pending => {
                // Safety: not moving `future2`.
                let future2 = unsafe { Pin::map_unchecked_mut(self, |s| &mut s.future2) };
                match future2.poll(ctx) {
                    Poll::Ready(value) => Poll::Ready(Err(value)),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

/// Returns a [`Future`] that get the next item from `stream`.
pub const fn next<S>(stream: S) -> Next<S> {
    Next { stream }
}

/// The [`Future`] behind [`either`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Next<S> {
    stream: S,
}

impl<S> Future for Next<S>
where
    S: Stream,
{
    type Output = Option<S::Item>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // Safety: not moving `stream`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.stream).poll_next(ctx) }
    }
}
