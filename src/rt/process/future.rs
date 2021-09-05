//! Module containing the implementation of the [`Process`] trait for
//! [`Future`]s.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{self, Poll};

use crate::rt::process::{Process, ProcessId, ProcessResult};
use crate::rt::{self, RuntimeRef};

/// A process that represent a [`Future`].
pub(crate) struct FutureProcess<Fut, RT> {
    future: Fut,
    /// We need to know whether we need to create thread-local or thread-safe
    /// waker.
    _phantom: PhantomData<RT>,
}

impl<Fut, RT> FutureProcess<Fut, RT> {
    pub(crate) const fn new(future: Fut) -> FutureProcess<Fut, RT> {
        FutureProcess {
            future,
            _phantom: PhantomData,
        }
    }
}

impl<Fut, RT> Process for FutureProcess<Fut, RT>
where
    Fut: Future<Output = ()>,
    RT: rt::Access,
{
    fn name(&self) -> &'static str {
        crate::actor::name::<Fut>()
    }

    fn run(self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef, pid: ProcessId) -> ProcessResult {
        // This is safe because we're not moving the future.
        let future = unsafe { Pin::map_unchecked_mut(self, |this| &mut this.future) };

        let waker = RT::new_task_waker(runtime_ref, pid);
        let mut task_ctx = task::Context::from_waker(&waker);
        match Future::poll(future, &mut task_ctx) {
            Poll::Ready(()) => ProcessResult::Complete,
            Poll::Pending => ProcessResult::Pending,
        }
    }
}
