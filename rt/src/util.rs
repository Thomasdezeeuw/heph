//! Module with various utilities.

use std::async_iter::AsyncIterator;
use std::future::Future;
use std::pin::Pin;
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
        // SAFETY: not moving `future1`.
        let future1 = unsafe { Pin::map_unchecked_mut(self.as_mut(), |s| &mut s.future1) };
        match future1.poll(ctx) {
            Poll::Ready(value) => Poll::Ready(Ok(value)),
            Poll::Pending => {
                // SAFETY: not moving `future2`.
                let future2 = unsafe { Pin::map_unchecked_mut(self, |s| &mut s.future2) };
                match future2.poll(ctx) {
                    Poll::Ready(value) => Poll::Ready(Err(value)),
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

/// Returns a [`Future`] that get the next item from `iter`.
pub const fn next<I>(iter: I) -> Next<I> {
    Next { iter }
}

/// The [`Future`] behind [`either`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Next<I> {
    iter: I,
}

impl<I> Future for Next<I>
where
    I: AsyncIterator,
{
    type Output = Option<I::Item>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving `iter`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.iter).poll_next(ctx) }
    }
}
