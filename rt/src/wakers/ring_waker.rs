//! [`task::Waker`] that wakes an [`a10::Ring`].

use std::task;

/// Returns a `task::Waker` that calls `sq.wake()` when called.
pub(crate) fn new(sq: a10::SubmissionQueue) -> task::Waker {
    let data = into_data(sq);
    let raw_waker = task::RawWaker::new(data, &WAKER_VTABLE);
    unsafe { task::Waker::from_raw(raw_waker) }
}

fn into_data(sq: a10::SubmissionQueue) -> *const () {
    // SAFETY: this is not safe. This only works because `a10::SubmissionQueue`
    // uses `Arc<_>`, which is a pointer underneath.
    unsafe { std::mem::transmute(sq) }
}

/// # Safety
///
/// `data` MUST be created by [`into_data`].
unsafe fn from_data(data: *const ()) -> a10::SubmissionQueue {
    // SAFETY: inverse of `into_data`, see that for more info.
    unsafe { std::mem::transmute(data) }
}

/// # Safety
///
/// Same safety requirement as [`from_data`]
unsafe fn from_data_ref<'a>(data: &'a *const ()) -> &'a a10::SubmissionQueue {
    // SAFETY: same as `from_data`.
    unsafe { &*(data as *const *const ()).cast() }
}

static WAKER_VTABLE: task::RawWakerVTable =
    task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, drop_wake_data);

unsafe fn clone_wake_data(data: *const ()) -> task::RawWaker {
    let sq = from_data_ref(&data).clone();
    let data = into_data(sq);
    task::RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn wake(data: *const ()) {
    from_data(data).wake();
}

unsafe fn wake_by_ref(data: *const ()) {
    from_data_ref(&data).wake();
}

unsafe fn drop_wake_data(data: *const ()) {
    drop(from_data(data));
}
