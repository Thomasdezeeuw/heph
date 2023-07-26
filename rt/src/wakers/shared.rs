//! Module containing the `task::Waker` implementation for thread-safe actors
//! and futures.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Weak;
use std::task;

use crate::process::ProcessId;
use crate::shared::RuntimeInternals;

/// Maximum number of runtimes supported.
const MAX_RUNTIMES: usize = 1 << MAX_RUNTIMES_BITS;
/// Number of most significate bits used for the [`WakersId`].
const MAX_RUNTIMES_BITS: usize = 8; // 256.
const WAKER_ID_SHIFT: usize = usize::BITS as usize - MAX_RUNTIMES_BITS;
const WAKER_ID_MASK: usize = (MAX_RUNTIMES - 1) << WAKER_ID_SHIFT;
const PID_MASK: usize = !WAKER_ID_MASK;

/// Type to create [`task::Waker`] for thread-safe actors and futures.
#[derive(Debug)]
pub(crate) struct Wakers {
    id: WakersId,
}

impl Wakers {
    /// Create a new `Wakers` waking processes in `internals`'s scheduler.
    pub(crate) fn new(internals: Weak<RuntimeInternals>) -> Wakers {
        /// Static used to determine unique indices into `RUNTIMES`.
        static IDS: AtomicU8 = AtomicU8::new(0);

        let id = IDS.fetch_add(1, Ordering::SeqCst);
        assert!(
            (id as usize) < MAX_RUNTIMES,
            "Created too many Heph `Runtime`s, maximum of {MAX_RUNTIMES}",
        );

        // SAFETY: this is safe because we are the only thread that has write access
        // to the given index. See documentation of `WAKERS` for more.
        unsafe { RUNTIMES[id as usize] = internals }
        Wakers { id: WakersId(id) }
    }

    /// Create a new [`task::Waker`] for the process with `pid`.
    pub(crate) fn new_task_waker(&self, pid: ProcessId) -> task::Waker {
        let data = WakerData::new(self.id, pid).into_raw_data();
        let raw_waker = task::RawWaker::new(data, &WAKER_VTABLE);
        // SAFETY: we follow the contract on `RawWaker`.
        unsafe { task::Waker::from_raw(raw_waker) }
    }
}

/// An id for a [`Wakers`].
///
/// This serves as index into `WAKERS`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
struct WakersId(u8);

/// Each coordinator, i.e. runtime, has a unique [`Wakers`] with unique
/// [`WakersId`] which is used as index into this array.
///
/// # Safety
///
/// Only [`Wakers::new`] may write to this array. After the initial write, no
/// more writes are allowed and the array element is read only.
///
/// Following the rules above means that there are no data races. The array can
/// only be indexed by [`WakersId`], which is only created by [`Waker::new`],
/// which ensures the waker is setup before returning the [`WakersId`]. This
/// ensures that only a single write happens to each element of the array. And
/// because after the initial write each element is read only there are no
/// further data races possible.
static mut RUNTIMES: [Weak<RuntimeInternals>; MAX_RUNTIMES] = [NO_RUNTIME; MAX_RUNTIMES];
// NOTE: this is only here because `NO_WAKER` is not `Copy`, thus
// `[None; MAX_THREADS]` doesn't work, but explicitly using a `const` does.
const NO_RUNTIME: Weak<RuntimeInternals> = Weak::new();

/// Get the internals for `waker_id`.
fn get(waker_id: WakersId) -> &'static Weak<RuntimeInternals> {
    // SAFETY: `WakersId` is only created by `Wakers::new`, which ensures its
    // valid. Furthermore `Wakers::new` ensures that `RUNTIMES[waker_id]` is
    // initialised and is read-only after that. See `RUNTIMES` docs for more.
    unsafe { &RUNTIMES[waker_id.0 as usize] }
}

/// Waker data passed to the [`task::Waker`] implementation.
///
/// # Layout
///
/// The [`MAX_RUNTIMES_BITS`] most significant bits are the [`WakersId`]. The
/// remaining bits are the [`ProcessId`], from which at least
/// `MAX_RUNTIMES_BITS` most significant bits are not used.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
struct WakerData(usize);

impl WakerData {
    /// Create new `WakerData`.
    fn new(waker_id: WakersId, pid: ProcessId) -> WakerData {
        let data = WakerData(pid.0 | ((waker_id.0 as usize) << WAKER_ID_SHIFT));
        debug_assert!(
            data.pid() == pid && data.waker_id() == waker_id,
            "`ProcessId` too large for `WakerData`"
        );
        data
    }

    /// Get the waker id.
    const fn waker_id(self) -> WakersId {
        // SAFETY: we know we won't truncate the waker id as it's an u8.
        #[allow(clippy::cast_possible_truncation)]
        WakersId(((self.0 & WAKER_ID_MASK) >> WAKER_ID_SHIFT) as u8)
    }

    /// Get the process id.
    const fn pid(self) -> ProcessId {
        // SAFETY: we know we won't truncate the pid, we checked in
        // `WakerData::new`.
        ProcessId(self.0 & PID_MASK)
    }

    /// Convert raw data from [`task::RawWaker`] into [`WakerData`].
    ///
    /// # Safety
    ///
    /// The caller must ensure the `data` is created using
    /// [`WakerData::into_raw_data`].
    unsafe fn from_raw_data(data: *const ()) -> WakerData {
        WakerData(data as usize)
    }

    /// Convert [`WakerData`] into raw data for [`task::RawWaker`].
    const fn into_raw_data(self) -> *const () {
        self.0 as *const ()
    }
}

// The two waker implementations below share the same `data`, see `WakerData`.
// The `WAKER_VTABLE` implementation schedules the process and wakes a worker
// thread, while `WAKER_VTABLE_NO_RING` only schedules the process and does
// **not** wake a worker thread. The latter can be used by I/O Futures that
// already wake a worker thread through the a10::Ring.
pub(super) use waker_vtable::WAKER_VTABLE;

const fn assert_copy<T: Copy>() {}

mod waker_vtable {
    use std::task;

    use crate::wakers::shared::{assert_copy, get, WakerData};

    /// Virtual table used by the `Waker` implementation.
    pub(crate) static WAKER_VTABLE: task::RawWakerVTable =
        task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, drop_wake_data);

    unsafe fn clone_wake_data(data: *const ()) -> task::RawWaker {
        assert_copy::<WakerData>();
        // Since the data is `Copy`, we just copy it.
        task::RawWaker::new(data, &WAKER_VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        // SAFETY: we received the data from the `RawWaker`, which doesn't modify
        // `data`.
        let data = unsafe { WakerData::from_raw_data(data) };
        if let Some(shared_internals) = get(data.waker_id()).upgrade() {
            shared_internals.mark_ready(data.pid());
            shared_internals.wake_workers(1);
        }
    }

    unsafe fn wake_by_ref(data: *const ()) {
        assert_copy::<WakerData>();
        // SAFETY: Since `WakerData` is `Copy` `wake` doesn't actually consume any
        // data, so we can just call it.
        unsafe { wake(data) };
    }

    unsafe fn drop_wake_data(_: *const ()) {
        assert_copy::<WakerData>();
        // Since `WakerData` is `Copy` we don't have to do anything.
    }
}

pub(super) mod waker_vtable_no_ring {
    //! [`task::Waker`] implementation that does **not** wake the worker thread.

    use std::task;

    use crate::wakers::shared::{assert_copy, get, WakerData};

    /// Virtual table used by the `Waker` implementation.
    pub(crate) static WAKER_VTABLE_NO_RING: task::RawWakerVTable =
        task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, drop_wake_data);

    pub(crate) unsafe fn clone_wake_data(data: *const ()) -> task::RawWaker {
        assert_copy::<WakerData>();
        // Since the data is `Copy`, we just copy it.
        task::RawWaker::new(data, &WAKER_VTABLE_NO_RING)
    }

    unsafe fn wake(data: *const ()) {
        // SAFETY: we received the data from the `RawWaker`, which doesn't modify
        // `data`.
        let data = unsafe { WakerData::from_raw_data(data) };
        if let Some(shared_internals) = get(data.waker_id()).upgrade() {
            shared_internals.mark_ready(data.pid());
            // NOTE: difference is that we don't wake any workers here.
        }
    }

    unsafe fn wake_by_ref(data: *const ()) {
        assert_copy::<WakerData>();
        // SAFETY: Since `WakerData` is `Copy` `wake` doesn't actually consume any
        // data, so we can just call it.
        unsafe { wake(data) };
    }

    unsafe fn drop_wake_data(_: *const ()) {
        assert_copy::<WakerData>();
        // Since `WakerData` is `Copy` we don't have to do anything.
    }
}
