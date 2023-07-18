//! Module with a scheduler for system processes.

use std::future::{pending, Future};
use std::pin::Pin;
use std::sync::Arc;
use std::{fmt, task};

use crate::wakers::{self, AtomicBitMap};

/// Type of a system [`Future`].
type SystemFuture = Pin<Box<dyn Future<Output = ()>>>;

/// Scheduler for system processes.
pub(crate) struct SystemScheduler {
    /// Collection of [`SystemFuture`]s.
    futures: Box<[SystemFuture]>,
    /// Bitmap indicating which `futures` are ready to run.
    ready: Arc<AtomicBitMap>,
}

impl SystemScheduler {
    /// Create a new `SystemScheduler`.
    pub(crate) fn new(futures: Box<[SystemFuture]>) -> SystemScheduler {
        let ready = AtomicBitMap::new(futures.len());
        SystemScheduler { futures, ready }
    }

    /// Runs all futures that are ready.
    pub(crate) fn run_ready(&mut self) {
        while let Some(idx) = self.ready.next_set() {
            let waker = wakers::new(self.ready.clone(), idx);
            let mut ctx = task::Context::from_waker(&waker);
            match self.futures[idx].as_mut().poll(&mut ctx) {
                task::Poll::Ready(()) => {
                    // Ensure we don't poll the future again.
                    self.futures[idx] = Box::pin(pending());
                }
                task::Poll::Pending => { /* Nothing to do. */ }
            }
        }
    }
}

impl fmt::Debug for SystemScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemScheduler")
            .field("futures", &self.futures.len())
            .field("ready", &self.ready)
            .finish()
    }
}
