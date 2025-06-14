//! A bad [`Future`] runtime implementation. This is just here for the examples,
//! don't actually use this. Consider the Heph-rt crate if you want a proper
//! implementation.

#![allow(dead_code)] // Not all examples use all functions.

use std::future::{Future, IntoFuture};
use std::pin::{pin, Pin};
use std::task::{self, Poll};

/// Block on the `future`, expecting polling `ring` to drive it forward.
pub fn block_on<Fut>(future: Fut) -> Fut::Output
where
    Fut: IntoFuture,
{
    let future = future.into_future();
    let mut future = pin!(future);

    loop {
        match poll_future(future.as_mut()) {
            Poll::Ready(output) => return output,
            Poll::Pending => {
                // We're just going to poll again.
                // This is why this implementation is bad.
            }
        }
    }
}

/// Block on the `future`, expecting polling `ring` to drive it forward.
pub fn block_on2<Fut1, Fut2>(future: Fut1, future2: Fut2)
where
    Fut1: IntoFuture,
    Fut2: IntoFuture,
{
    let future1 = pin!(future.into_future());
    let mut future1 = Some(future1);
    let future2 = pin!(future2.into_future());
    let mut future2 = Some(future2);

    loop {
        if let Some(fut1) = future1.as_mut() {
            if poll_future(fut1.as_mut()).is_ready() {
                future1 = None;
            }
        }
        if let Some(fut2) = future2.as_mut() {
            if poll_future(fut2.as_mut()).is_ready() {
                future2 = None;
            }
        }

        if future1.is_none() && future2.is_none() {
            return;
        }
    }
}

/// Since we only have a single future we don't need to be awoken.
fn poll_future<Fut>(fut: Pin<&mut Fut>) -> Poll<Fut::Output>
where
    Fut: Future,
{
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(&waker);
    fut.poll(&mut ctx)
}
