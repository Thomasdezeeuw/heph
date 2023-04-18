//! Waker implementation for the coordinator.
//!
//! # Implementation
//!
//! The implementation is fairly simple. All it does is set a bit in an
//! [`AtomicBitMap`] contained in an [`Arc`].

use std::sync::Arc;
use std::{ptr, task};

use crate::coordinator::bitmap::AtomicBitMap;

/// Maximum number of wakers this module supports.
pub(crate) const MAX_WAKERS: usize = 1 << PTR_BITS_UNUSED;
/// Number of bits we expect a 64 bit pointer to not used, leaving them for us
/// to fill with our index (into `AtomicBitMap`).
const PTR_BITS_UNUSED: usize = 16;
/// Amount of bits to shift to not overwrite the pointer address.
const PTR_DATA_SHIFT: usize = usize::BITS as usize - PTR_BITS_UNUSED;
/// Mask to get the data from a pointer.
const DATA_MASK: usize = ((1 << PTR_BITS_UNUSED) - 1) << PTR_DATA_SHIFT;
/// Mask to get the pointer to the `AtomicBitMap`.
const PTR_MASK: usize = (1 << PTR_DATA_SHIFT) - 1;

pub(super) fn new(bitmap: Arc<AtomicBitMap>, id: usize) -> task::Waker {
    let data = into_data_ptr(bitmap, id);
    let raw_waker = task::RawWaker::new(data, &WAKER_VTABLE);
    unsafe { task::Waker::from_raw(raw_waker) }
}

/// # Panics
///
/// This will panic if the capacity of `bitmap` is smaller than `id`. `id` must
/// be smallar then [`MAX_WAKERS`].
fn into_data_ptr(bitmap: Arc<AtomicBitMap>, id: usize) -> *const () {
    // Check the input is valid.
    assert!(bitmap.capacity() >= id);
    assert!(id <= MAX_WAKERS);

    // This is a "fat" pointer, a pointer to `AtomicBitMap` and a length.
    let bitmap_ptr = Arc::into_raw(bitmap);
    // This will point to the start of the `AtomicBitMap` as is "thin".
    let bitmap_start = bitmap_ptr.cast::<()>();
    // Ensure we have bit to put our `id`.
    assert!(bitmap_start as usize & PTR_BITS_UNUSED == 0);
    // Squash the pointer and our `id` together.
    ((bitmap_start as usize) & (id << PTR_DATA_SHIFT)) as *const ()
}

static WAKER_VTABLE: task::RawWakerVTable =
    task::RawWakerVTable::new(clone_wake_data, wake, wake_by_ref, drop_wake_data);

unsafe fn clone_wake_data(data: *const ()) -> task::RawWaker {
    let (bitmap_ptr, _) = data_as_raw_ptr(data);
    Arc::increment_strong_count(bitmap_ptr);
    // After we incremented the strong count we can reuse the same data.
    task::RawWaker::new(data, &WAKER_VTABLE)
}

unsafe fn wake(data: *const ()) {
    let (bitmap, id) = from_data_ptr(data);
    bitmap.set(id);
}

unsafe fn wake_by_ref(data: *const ()) {
    let (bitmap_ptr, id) = data_as_raw_ptr(data);
    let bitmap = &*bitmap_ptr;
    bitmap.set(id);
}

unsafe fn drop_wake_data(data: *const ()) {
    drop(from_data_ptr(data));
}

/// # Safety
///
/// `data` MUST be created by [`into_data_ptr`].
unsafe fn from_data_ptr(data: *const ()) -> (Arc<AtomicBitMap>, usize) {
    let (bitmap_ptr, id) = data_as_raw_ptr(data);
    (Arc::from_raw(bitmap_ptr), id)
}

/// Returns a raw pointer to the `AtomicBitMap` inside of an `Arc`.
///
/// # Safety
///
/// `data` MUST be created by [`into_data_ptr`].
unsafe fn data_as_raw_ptr(data: *const ()) -> (*const AtomicBitMap, usize) {
    // SAFETY: the caller must ensure that `data` is created using
    // `into_data_ptr`. That guarantees us two things, 1) `id` is valid and 2)
    // that the pointer is valid and the bitmap has enough capacity for the
    // `id`.
    // The above guarantees ensure that calling `min_bitmap_size` results in a
    // bitmap that has at least enough capacity that we can set the `id`-th bit.
    // The returned pointer might be a shorter than the true length of
    // `AtomicBitMap`, but we can work with that.
    let id = data as usize & DATA_MASK;
    let bitmap_start = (data as usize & PTR_MASK) as *const ();
    let bitmap_size = min_bitmap_size(id);
    let bitmap_ptr = ptr::from_raw_parts(bitmap_start, bitmap_size);
    (bitmap_ptr, id)
}

/// Returns the minimum bitmap size such that `id` can be set.
fn min_bitmap_size(id: usize) -> usize {
    let mut bitmap_size = id / usize::BITS as usize;
    if (id % usize::BITS as usize) != 0 {
        bitmap_size += 1;
    }
    bitmap_size
}
