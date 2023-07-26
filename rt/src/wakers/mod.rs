//! Wakers implementations.

use std::future::Future;
use std::mem::{replace, ManuallyDrop};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{self, Poll};

use crossbeam_channel::Sender;
use log::{error, trace};

use crate::process::ProcessId;

pub(crate) mod shared;
#[cfg(test)]
mod tests;

/// Maximum number of wakers this implementation supports.
pub(crate) const MAX_WAKERS: usize = 1 << PTR_BITS_UNUSED;
/// Number of bits we expect a 64 bit pointer to not use, leaving them for us to
/// fill with our index (into [`WakerData::slot`]).
const PTR_BITS_UNUSED: usize = 10;
/// Amount of bits to shift to not overwrite the pointer address.
const PTR_DATA_SHIFT: usize = usize::BITS as usize - PTR_BITS_UNUSED;
/// Mask to get the data from a pointer.
const DATA_MASK: usize = ((1 << PTR_BITS_UNUSED) - 1) << PTR_DATA_SHIFT;
/// Mask to get the pointer to [`WakerData`].
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
        #[allow(clippy::declare_interior_mutable_const)]
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

    /// Clone `data` to another owned version of the data.
    ///
    /// # Safety
    ///
    /// `data` MUST be created by [`WakerData::into_data_ptr`].
    unsafe fn clone_data(data: *const ()) -> *const () {
        // Since we don't want to drop the `Arc`, neither the one we crate from
        // `data` or the one we clone, we put both in a `ManuallyDrop` wrapper and
        // don't drop them. After the clone we can reuse `data`.
        let waker_data = ManuallyDrop::new(WakerData::from_data_ptr(data).0);
        let _: ManuallyDrop<Arc<WakerData>> = waker_data.clone();
        // `data` remains the same, so we can just return it. All we needed to
        // do was increment the `Arc`'s strong count.
        data
    }

    /// Drop `data`.
    ///
    /// # Safety
    ///
    /// `data` MUST be created by [`WakerData::into_data_ptr`].
    unsafe fn drop_data(data: *const ()) {
        drop(WakerData::from_data_ptr(data));
    }

    /// Wake the process with `pid`.
    fn wake(&self, pid: ProcessId) {
        self.wake_no_ring(pid);
        trace!(pid = pid.0; "waking worker thread to run process");
        self.sq.wake();
    }

    /// Wake the process with `pid`, but do **not** wake the thread.
    fn wake_no_ring(&self, pid: ProcessId) {
        trace!(pid = pid.0; "waking process");
        if let Err(err) = self.notifications.try_send(pid) {
            error!("unable to send wake up notification: {err}");
        }
    }
}

/// Overwrite `$ctx` with a [`task::Context`] that does **not** wake the worker
/// thread. This will only work if the `Future` wake the worker thread in some
/// other way, for example I/O futures will wake the thread via a10::Ring
/// (io_uring).
macro_rules! no_ring_ctx {
    ($ctx: ident) => {
        let task_waker;
        let mut task_ctx;
        if let Some(waker) = $crate::wakers::create_no_ring_waker($ctx) {
            task_waker = waker;
            task_ctx = std::task::Context::from_waker(&task_waker);
        } else {
            task_ctx = std::task::Context::from_waker($ctx.waker());
        };
        let $ctx = &mut task_ctx;
    };
}

pub(crate) use no_ring_ctx;

/// [`Future`] that uses [`no_ring_ctx`] internally.
pub(crate) struct NoRing<Fut>(pub(crate) Fut);

impl<Fut: Future> Future for NoRing<Fut> {
    type Output = Fut::Output;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        no_ring_ctx!(ctx);
        // SAFETY: not moving the `Future`.
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.0) }.poll(ctx)
    }
}

/// Create a `task::Waker` that does **not** wake the worker thread. Returns
/// `None` if the ctx doesn't use a Heph provided waker implementation.
#[allow(clippy::needless_pass_by_ref_mut)] // Matching `Future::poll` signature.
pub(crate) fn create_no_ring_waker(ctx: &mut task::Context<'_>) -> Option<task::Waker> {
    let raw_waker = ctx.waker().as_raw();
    let vtable = *raw_waker.vtable();
    // SAFETY: the `data` comes from a waker and when check which waker
    // implementation is used.
    unsafe {
        Some(task::Waker::from_raw(if vtable == WAKER_VTABLE {
            waker_vtable_no_ring::clone_wake_data(raw_waker.data())
        } else if vtable == shared::WAKER_VTABLE {
            shared::waker_vtable_no_ring::clone_wake_data(raw_waker.data())
        } else {
            return None;
        }))
    }
}

// The two waker implementations below share the same `data`, see `WakerData`.
// The `WAKER_VTABLE` implementation schedules the process and wakes the worker
// thread, while `WAKER_VTABLE_NO_RING` only schedules the process and does
// **not** wake the worker thread. The latter can be used by I/O Futures that
// already wake the worker thread through the a10::Ring.
use waker_vtable::WAKER_VTABLE;

mod waker_vtable {
    use std::task;

    use crate::wakers::WakerData;

    /// Virtual table used by the `task::Waker` implementation of [`Wakers`].
    pub(crate) static WAKER_VTABLE: task::RawWakerVTable =
        task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, WakerData::drop_data);

    unsafe fn clone_wake_data(data: *const ()) -> task::RawWaker {
        let data = WakerData::clone_data(data);
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
}

mod waker_vtable_no_ring {
    //! [`task::Waker`] implementation that does **not** wake the worker thread.

    use std::task;

    use crate::wakers::WakerData;

    /// Virtual table used by the `task::Waker` implementation of [`Wakers`].
    static WAKER_VTABLE_NO_RING: task::RawWakerVTable =
        task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, WakerData::drop_data);

    pub(crate) unsafe fn clone_wake_data(data: *const ()) -> task::RawWaker {
        let data = WakerData::clone_data(data);
        task::RawWaker::new(data, &WAKER_VTABLE_NO_RING)
    }

    unsafe fn wake(data: *const ()) {
        let (data, pid) = WakerData::from_data_ptr(data);
        data.wake_no_ring(pid);
    }

    unsafe fn wake_by_ref(data: *const ()) {
        let (data, pid) = WakerData::from_data_ptr_ref(data);
        data.wake_no_ring(pid);
    }
}
