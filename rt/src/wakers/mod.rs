//! Wakers implementations.

use std::mem::{replace, ManuallyDrop};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task;

use crossbeam_channel::Sender;
use log::{error, trace};

use crate::process::ProcessId;

pub(crate) mod shared;
#[cfg(test)]
mod tests;

/// Maximum number of wakers this implementation supports.
pub(crate) const MAX_WAKERS: usize = 1 << PTR_BITS_UNUSED;
/// Number of bits we expect a 64 bit pointer to not use, leaving them for us to
/// fill with our index (into [`AtomicBitMap`]).
const PTR_BITS_UNUSED: usize = 10;
/// Amount of bits to shift to not overwrite the pointer address.
const PTR_DATA_SHIFT: usize = usize::BITS as usize - PTR_BITS_UNUSED;
/// Mask to get the data from a pointer.
const DATA_MASK: usize = ((1 << PTR_BITS_UNUSED) - 1) << PTR_DATA_SHIFT;
/// Mask to get the pointer to the `AtomicBitMap`.
const PTR_MASK: usize = (1 << PTR_DATA_SHIFT) - 1;

#[derive(Debug)]
pub(crate) struct Wakers {
    notifications: Sender<ProcessId>,
    sq: a10::SubmissionQueue,
    current_data: Arc<WakerData>,
    current_slot: usize,
}

impl Wakers {
    /// Create a new `Wakers` sending `ProcessId`s into `notifications` and
    /// waking `sq` on wake up.
    pub(crate) fn new(notifications: Sender<ProcessId>, sq: a10::SubmissionQueue) -> Wakers {
        let current_data = WakerData::empty(notifications.clone(), sq.clone());
        Wakers {
            notifications,
            sq,
            current_data,
            current_slot: 0,
        }
    }

    /// Create a new `task::Waker`.
    ///
    /// # Notes
    ///
    /// Waking data is **not** reused between `pid`s, so it's easier to clone an
    /// existing `task::Waker` then to call this a second time.
    pub(crate) fn new_task_waker(&mut self, pid: ProcessId) -> task::Waker {
        let slot = self.current_slot;

        self.current_slot += 1;
        let data = if self.current_slot >= MAX_WAKERS {
            self.current_slot = 0;
            replace(
                &mut self.current_data,
                WakerData::empty(self.notifications.clone(), self.sq.clone()),
            )
        } else {
            self.current_data.clone()
        };

        // Relaxed is fine as it won't be read by anyone until the `task::Waker`
        // that is return by this function is used.
        data.slots[slot].store(pid.0, Ordering::Relaxed);

        let data = unsafe { data.into_data_ptr(slot) };
        unsafe { task::Waker::from_raw(task::RawWaker::new(data, &WAKER_VTABLE)) }
    }
}

/// Data that the [`WAKER_VTABLE`] implementation has access to.
#[derive(Debug)]
struct WakerData {
    /// Indication which process is ready to run.
    notifications: Sender<ProcessId>,
    /// A way to wake up the thread.
    sq: a10::SubmissionQueue,
    /// `task::Waker` only gives us a single pointer worth of data. From that we
    /// need `ProcessId` and pointer (or two) to actually schedule the process.
    /// Since all of that doesn't fit into a single pointer we use this `slots`
    /// mapping. `slots` maps indices (packed into the `data` pointer) to
    /// `ProcessId`s.
    ///
    /// # Safety
    ///
    /// [`Wakers::new_task_waker`] may write to this ONCE, after that the values
    /// are read only, which allows us to `Relaxed` ordering.
    slots: [AtomicUsize; MAX_WAKERS],
}

impl WakerData {
    /// Create empty `WakerData`.
    fn empty(notifications: Sender<ProcessId>, sq: a10::SubmissionQueue) -> Arc<WakerData> {
        const ZERO_PID: AtomicUsize = AtomicUsize::new(0);
        Arc::new(WakerData {
            notifications,
            sq,
            slots: [ZERO_PID; MAX_WAKERS],
        })
    }

    /// Turn itself into `task::Waker` `data` waking process in `slot`.
    ///
    /// # Safety
    ///
    /// If the returned `data` pointer is not converted using
    /// [`WakerData::from_data_ptr`] it will leak the `Arc<Self>`.
    unsafe fn into_data_ptr(self: Arc<Self>, slot: usize) -> *const () {
        // Squash the pointer and our `slot` together.
        let ptr = Arc::into_raw(self);
        let data = ((ptr as usize) | (slot << PTR_DATA_SHIFT)) as *const ();
        // Ensure we can extra the current pointer from the squashed data.
        assert!(data as usize & PTR_MASK == ptr as usize);
        data
    }

    /// # Safety
    ///
    /// `data` MUST be created by [`WakerData::into_data_ptr`] and MUST be
    /// owned.
    unsafe fn from_data_ptr(data: *const ()) -> (Arc<WakerData>, ProcessId) {
        let (ptr, pid) = WakerData::from_data_ptr_ref(data);
        // SAFETY: this is safe because the caller must ensure that the `data`
        // was created by `WakerData::into_data_ptr`, which ensures that is
        // contains a valid pointer to `WakerData`. Furthermore the caller must
        // ensure we own the `data`.
        let data = unsafe { Arc::from_raw(ptr) };
        (data, pid)
    }

    /// Returns a raw pointer to the `WakerData` inside of an `Arc`.
    ///
    /// # Safety
    ///
    /// `data` MUST be created by [`WakerData::into_data_ptr`].
    unsafe fn from_data_ptr_ref<'a>(data: *const ()) -> (&'a WakerData, ProcessId) {
        let slot = (data as usize & DATA_MASK) >> PTR_DATA_SHIFT;
        let ptr = (data as usize & PTR_MASK) as *const WakerData;
        // SAFETY: this is safe because the caller must ensure that the `data`
        // was created by `WakerData::into_data_ptr`, which ensures that is
        // contains a valid pointer to `WakerData`.
        let data = unsafe { &*ptr };
        // Relaxed ok here as the pid isn't changed.
        let pid = data.slots[slot].load(Ordering::Relaxed);
        (data, ProcessId(pid))
    }

    /// Wake the process with `pid`.
    fn wake(&self, pid: ProcessId) {
        trace!(pid = pid.0; "waking process");
        if let Err(err) = self.notifications.try_send(pid) {
            error!("unable to send wake up notification: {err}");
            return;
        }
        self.sq.wake();
    }
}

/// Virtual table used by the `task::Waker` implementation of [`Wakers`].
static WAKER_VTABLE: task::RawWakerVTable =
    task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, drop_wake_data);

unsafe fn clone_wake_data(data: *const ()) -> task::RawWaker {
    // Since we don't want to drop the `Arc`, neither the one we crate from
    // `data` or the one we clone, we put both in a `ManuallyDrop` wrapper and
    // don't drop them. After the clone we can reuse `data`.
    let waker_data = ManuallyDrop::new(WakerData::from_data_ptr(data).0);
    let _: ManuallyDrop<Arc<WakerData>> = waker_data.clone();
    task::RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn wake(data: *const ()) {
    let (data, pid) = WakerData::from_data_ptr(data);
    data.wake(pid);
}

unsafe fn wake_by_ref(data: *const ()) {
    let (data, pid) = WakerData::from_data_ptr_ref(data);
    data.wake(pid);
}

unsafe fn drop_wake_data(data: *const ()) {
    drop(WakerData::from_data_ptr(data));
}
