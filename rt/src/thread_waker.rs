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
    /// polling, avoiding a system call when the thread is not polling.
    polling_status: AtomicU8,
    waker: mio::Waker,
}

/// Not currently polling.
const NOT_POLLING: u8 = 0;
/// Currently polling.
const IS_POLLING: u8 = 1;
/// We add 2 to `IS_POLLING` to ensure that we don't go from `NOT_POLLING` to
/// `IS_POLLING` (by adding 1).
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
        // Safety: this needs to sync with the `store(Release)` in
        // `mark_polling`, hence `Acquire` is needed.
        if self.polling_status.load(Ordering::Acquire) != IS_POLLING {
            return Ok(false);
        }

        // Safety: this needs to sync with the `store(Release)` in
        // `mark_polling`, hence `AcqRel` is needed.
        if self.polling_status.fetch_add(WAKING, Ordering::AcqRel) == IS_POLLING {
            self.waker.wake().map(|()| true)
        } else {
            Ok(false)
        }
    }

    /// Mark the thread as currently polling (or not).
    pub(crate) fn mark_polling(&self, is_polling: bool) {
        let status = if is_polling { IS_POLLING } else { NOT_POLLING };
        // Safety: this needs to sync with the `load` and `fetch_add` in `wake`,
        // thus `Release` is needed.
        // NOTE: don't lower the strength of this ordering as will not generate
        // the correct assembly.
        self.polling_status.store(status, Ordering::SeqCst);
    }
}
