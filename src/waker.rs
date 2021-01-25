use std::sync::atomic::{AtomicBool, Ordering};
use std::task;

use parking_lot::{const_rwlock, RwLock, RwLockUpgradableReadGuard};

/// Registration of a [`task::Waker`].
pub(crate) struct WakerRegistration {
    /// This will be `true` if this waker needs to be awoken, `false` otherwise.
    needs_wakeup: AtomicBool,
    /// The actual waking mechanism.
    waker: RwLock<Option<task::Waker>>,
}

impl WakerRegistration {
    /// Create a new empty registration.
    pub(crate) const fn new() -> WakerRegistration {
        WakerRegistration {
            needs_wakeup: AtomicBool::new(false),
            waker: const_rwlock(None),
        }
    }

    /// Register `waker`.
    pub(crate) fn register(&self, waker: &task::Waker) -> bool {
        let stored_waker = self.waker.upgradable_read();
        if let Some(stored_waker) = &*stored_waker {
            if stored_waker.will_wake(waker) {
                self.needs_wakeup.store(true, Ordering::SeqCst);
                return false;
            }
        }

        let waker = Some(waker.clone());
        let mut stored_waker = RwLockUpgradableReadGuard::upgrade(stored_waker);
        *stored_waker = waker;
        drop(stored_waker);

        self.needs_wakeup.store(true, Ordering::SeqCst);
        true
    }

    /// Wake the waker registered, if required.
    pub(crate) fn wake(&self) {
        if !self.needs_wakeup.load(Ordering::SeqCst) {
            // Receiver doesn't need a wake-up.
            return;
        }

        // Mark that we've woken and after actually do the waking.
        if self.needs_wakeup.swap(false, Ordering::SeqCst) {
            if let Some(waker) = &*self.waker.read() {
                waker.wake_by_ref();
            }
        }
    }
}
