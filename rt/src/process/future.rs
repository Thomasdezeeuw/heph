//! Module containing the implementation of the [`Process`] trait for
//! [`Future`]s.

use std::future::Future;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{self, Poll};

use log::error;

use crate::process::{panic_message, Process, ProcessId, ProcessResult};
use crate::{self as rt, RuntimeRef};

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
        name::<Fut>()
    }

    fn run(self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef, pid: ProcessId) -> ProcessResult {
        // This is safe because we're not moving the future.
        let future = unsafe { Pin::map_unchecked_mut(self, |this| &mut this.future) };

        let waker = RT::new_task_waker(runtime_ref, pid);
        let mut task_ctx = task::Context::from_waker(&waker);

        match catch_unwind(AssertUnwindSafe(|| Future::poll(future, &mut task_ctx))) {
            Ok(Poll::Ready(())) => ProcessResult::Complete,
            Ok(Poll::Pending) => ProcessResult::Pending,
            Err(panic) => {
                let msg = panic_message(&*panic);
                error!("future '{}' panicked at '{}'", name::<Fut>(), msg);
                ProcessResult::Complete
            }
        }
    }
}

fn name<Fut>() -> &'static str {
    heph::actor::name::<Fut>()
}
