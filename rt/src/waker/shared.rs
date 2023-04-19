//! Module containing the `task::Waker` implementation for thread-safe actors
//! and futures.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Weak;
use std::task;

use crate::shared::RuntimeInternals;
use crate::{ptr_as_usize, ProcessId};

/// Maximum number of runtimes supported.
const MAX_RUNTIMES: usize = 1 << MAX_RUNTIMES_BITS;
/// Number of most significate bits used for the [`WakerId`].
#[cfg(not(any(test, feature = "test")))]
const MAX_RUNTIMES_BITS: usize = 0; // 1.
#[cfg(any(test, feature = "test"))]
const MAX_RUNTIMES_BITS: usize = 8; // 256.
const WAKER_ID_SHIFT: usize = usize::BITS as usize - MAX_RUNTIMES_BITS;
const WAKER_ID_MASK: usize = (MAX_RUNTIMES - 1) << WAKER_ID_SHIFT;
const PID_MASK: usize = !WAKER_ID_MASK;

/// An id for a waker.
///
/// Returned by [`init`] and used in [`new`] to create a new [`task::Waker`].
///
/// This serves as index into `WAKERS`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub(crate) struct WakerId(u8);

/// Each coordinator has a unique [`WakerId`] which is used as index into this
/// array.
///
/// # Safety
///
/// Only [`init`] may write to this array. After the initial write, no more
/// writes are allowed and the array element is read only. To get a
/// [`task::Waker`] use the [`new`] function.
///
/// Following the rules above means that there are no data races. The array can
/// only be indexed by [`WakerId`], which is only created by [`init`], which
/// ensures the waker is setup before returning the [`WakerId`]. This ensures
/// that only a single write happens to each element of the array. And because
/// after the initial write each element is read only there are no further data
/// races possible.
static mut RUNTIMES: [Weak<RuntimeInternals>; MAX_RUNTIMES] = [NO_RUNTIME; MAX_RUNTIMES];
// NOTE: this is only here because `NO_WAKER` is not `Copy`, thus
// `[None; MAX_THREADS]` doesn't work, but explicitly using a `const` does.
const NO_RUNTIME: Weak<RuntimeInternals> = Weak::new();

/// Initialise a new waker.
///
/// This returns a [`WakerId`] which can be used to create a new [`task::Waker`]
/// using [`new`].
pub(crate) fn init_shared_waker(internals: Weak<RuntimeInternals>) -> WakerId {
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
    WakerId(id)
}

/// Create a new [`task::Waker`].
///
/// [`init`] must be called before calling this function to get a [`WakerId`].
pub(crate) fn new_shared_task_waker(waker_id: WakerId, pid: ProcessId) -> task::Waker {
    let data = WakerData::new(waker_id, pid).into_raw_data();
    let raw_waker = task::RawWaker::new(data, &WAKER_VTABLE);
    // SAFETY: we follow the contract on `RawWaker`.
    unsafe { task::Waker::from_raw(raw_waker) }
}

/// Get the internals for `waker_id`.
fn get(waker_id: WakerId) -> &'static Weak<RuntimeInternals> {
    // SAFETY: `WakerId` is only created by `init`, which ensures its valid.
    // Furthermore `init` ensures that `RUNTIMES[waker_id]` is initialised and
    // is read-only after that. See `RUNTIMES` documentation for more.
    unsafe { &RUNTIMES[waker_id.0 as usize] }
}

/// Waker data passed to the [`task::Waker`] implementation.
///
/// # Layout
///
/// The [`MAX_RUNTIMES_BITS`] most significant bits are the [`WakerId`]. The
/// remaining bits are the [`ProcessId`], from which at least
/// `MAX_RUNTIMES_BITS` most significant bits are not used.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
struct WakerData(usize);

impl WakerData {
    /// Create new `WakerData`.
    fn new(waker_id: WakerId, pid: ProcessId) -> WakerData {
        let data = WakerData(pid.0 | ((waker_id.0 as usize) << WAKER_ID_SHIFT));
        debug_assert!(
            data.pid() == pid && data.waker_id() == waker_id,
            "`ProcessId` too large for `WakerData`"
        );
        data
    }

    /// Get the waker id.
    const fn waker_id(self) -> WakerId {
        // SAFETY: we know we won't truncate the waker id as it's an u8.
        #[allow(clippy::cast_possible_truncation)]
        WakerId(((self.0 & WAKER_ID_MASK) >> WAKER_ID_SHIFT) as u8)
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
    /// This doesn't check if the provided `data` is valid, the caller is
    /// responsible for this.
    const unsafe fn from_raw_data(data: *const ()) -> WakerData {
        WakerData(ptr_as_usize(data))
    }

    /// Convert [`WakerData`] into raw data for [`task::RawWaker`].
    const fn into_raw_data(self) -> *const () {
        self.0 as *const ()
    }
}

/// Virtual table used by the `Waker` implementation.
static WAKER_VTABLE: task::RawWakerVTable =
    task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, drop_wake_data);

fn assert_copy<T: Copy>() {}

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