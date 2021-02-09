//! Module with the thread waking mechanism.

use std::io;
use std::sync::atomic::{AtomicU8, Ordering};

/// Mechanism to wake up a thread from [polling].
///
/// [polling]: mio::Poll::poll
#[derive(Debug)]
pub(crate) struct ThreadWaker {
    /// If the worker thread is polling (without a timeout) we need to wake it
    /// to ensure it doesn't poll for ever. This status is used to determine
    /// whether or not we need use [`mio::Waker`] to wake the thread from
    /// polling.
    polling_status: AtomicU8,
    waker: mio::Waker,
}

/// Not currently polling.
const NOT_POLLING: u8 = 0;
/// Currently polling.
const IS_POLLING: u8 = 1;
/// Going to wake the polling thread.
const WAKING: u8 = 2;

impl ThreadWaker {
    /// Create a new `ThreadWaker`.
    pub(crate) const fn new(waker: mio::Waker) -> ThreadWaker {
        ThreadWaker {
            polling_status: AtomicU8::new(NOT_POLLING),
            waker,
        }
    }

    /// Wake up the thread if it's not currently polling. Returns `true` if the
    /// thread is awoken, `false` otherwise.
    pub(crate) fn wake(&self) -> io::Result<bool> {
        // If the thread is currently polling we're going to wake it. To avoid
        // additional calls to `Waker::wake` we use compare_exchange and let
        // only a single call to `Thread::wake` wake the thread.
        let res = self
            .polling_status
            // We don't care about the result so `Relaxed` ordering is fine.
            .compare_exchange(IS_POLLING, WAKING, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok();
        if res {
            self.waker.wake().map(|()| true)
        } else {
            Ok(false)
        }
    }

    /// Mark the thread as currently polling (or not).
    pub(crate) fn mark_polling(&self, is_polling: bool) {
        let status = if is_polling { IS_POLLING } else { NOT_POLLING };
        self.polling_status.store(status, Ordering::Release);
    }
}
