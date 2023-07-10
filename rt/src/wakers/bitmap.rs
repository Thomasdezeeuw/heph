//! Atomic bit map.
//!
//! See [`AtomicBitMap`].

use std::fmt;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Variable sized atomic bitmap.
#[repr(transparent)]
pub(crate) struct AtomicBitMap {
    data: [AtomicUsize],
}

impl AtomicBitMap {
    /// Create a new `AtomicBitMap`.
    ///
    /// # Notes
    ///
    /// `entries` is a minimum number of slots, use `capacity` to determine the
    /// actual number of bits available.
    pub(crate) fn new(entries: usize) -> Arc<AtomicBitMap> {
        let mut size = entries / usize::BITS as usize;
        if (entries % usize::BITS as usize) != 0 {
            size += 1;
        }
        let arc: Arc<[MaybeUninit<AtomicUsize>]> = Arc::new_zeroed_slice(size);
        // SAFETY: This cast does two things:
        //  * `[MaybeUninit<AtomicUsize>]` -> `[AtomicUsize]`: this is safe
        //    because all zeroes is valid for `AtomicUsize`.
        //  * `Arc<[AtomicUsize]>` -> `Arc<AtomicBitMap>`: this is safe because
        //    of the use of `repr(transparent)` on `AtomicBitMap` ensuring it
        //    has the same layout as `[AtomicUsize]`.
        unsafe { Arc::from_raw(Arc::into_raw(arc) as _) }
    }

    /// Returns the number of indices the bitmap can manage.
    pub(crate) const fn capacity(&self) -> usize {
        self.data.len() * usize::BITS as usize
    }

    /// Returns the index of the set slot, or `None`.
    pub(crate) fn next_set(&self) -> Option<usize> {
        for (idx, data) in self.data.iter().enumerate() {
            let mut value = data.load(Ordering::Relaxed);
            let mut i = value.trailing_zeros();
            while i < usize::BITS {
                // Attempt to unset the bit, claiming the slot.
                value = data.fetch_and(!(1 << i), Ordering::SeqCst);
                // Another thread could have attempted to unset the same bit
                // we're setting, so we need to make sure we actually set the
                // bit (i.e. check if was set in the previous state).
                if is_set(value, i as usize) {
                    return Some((idx * usize::BITS as usize) + i as usize);
                }
                i += (value >> i).trailing_zeros();
            }
        }
        None
    }

    /// Set the bit at `index`.
    pub(crate) fn set(&self, index: usize) {
        let idx = index / usize::BITS as usize;
        let n = index % usize::BITS as usize;
        let old_value = self.data[idx].fetch_or(1 << n, Ordering::SeqCst);
        debug_assert!(!is_set(old_value, n));
    }
}

/// Returns true if bit `n` is set in `value`. `n` is zero indexed, i.e. must be
/// in the range 0..usize::BITS (64).
const fn is_set(value: usize, n: usize) -> bool {
    ((value >> n) & 1) == 1
}

impl fmt::Debug for AtomicBitMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const WIDTH: usize = usize::BITS as usize;
        for data in &self.data {
            let value = data.load(Ordering::Relaxed);
            write!(f, "{value:0WIDTH$b}")?;
        }
        Ok(())
    }
}
