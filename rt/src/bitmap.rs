//! Atomic bitmap.
//!
//! See [`AtomicBitMap`].

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    /// `entries` is a minimum number of slots, use [`AtomicBitMap::capacity`]
    /// to determine the actual number of bits available.
    pub(crate) fn new(entries: usize) -> Arc<AtomicBitMap> {
        let mut size = entries / usize::BITS as usize;
        if !entries.is_multiple_of(usize::BITS as usize) {
            size += 1;
        }
        let mut vec = Vec::with_capacity(size);
        vec.resize_with(size, || AtomicUsize::new(0));
        let bitmap: Arc<[AtomicUsize]> = Arc::from(vec);
        unsafe { std::mem::transmute(bitmap) }
    }

    /// Returns the number of indices the bitmap can manage.
    #[allow(dead_code)]
    pub(crate) const fn capacity(&self) -> usize {
        self.data.len() * usize::BITS as usize
    }

    /// Returns the index of the available slot, or `None`.
    pub(crate) fn next_set(&self) -> Option<usize> {
        for (idx, data) in self.data.iter().enumerate() {
            // SAFETY: Relaxed ordering is acceptable here because when we
            // actually attempt to set the bit below (the fetch_and) we use the
            // correct AcqRel ordering.
            let mut value = data.load(Ordering::Relaxed);
            let mut i = value.trailing_zeros();
            while i < usize::BITS {
                // Attempt to set the bit, claiming the slot.
                value = data.fetch_and(!(1 << i), Ordering::AcqRel);
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

    /// Set bit at `index` to `value`.
    pub(crate) fn set(&self, index: usize) {
        let idx = index / usize::BITS as usize;
        let n = index % usize::BITS as usize;
        if let Some(data) = self.data.get(idx) {
            _ = data.fetch_or(1 << n, Ordering::AcqRel);
        }
    }
}

/// Returns true if bit `n` is not set in `value`. `n` is zero indexed, i.e.
/// must be in the range 0..usize::BITS (64).
const fn is_set(value: usize, n: usize) -> bool {
    ((value >> n) & 1) != 0
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Barrier};
    use std::thread;

    use crate::AtomicBitMap;

    #[test]
    fn setting_and_unsetting_one() {
        setting_and_unsetting(64)
    }

    #[test]
    fn setting_and_unsetting_two() {
        setting_and_unsetting(128)
    }

    #[test]
    fn setting_and_unsetting_three() {
        setting_and_unsetting(192)
    }

    #[test]
    fn setting_and_unsetting_four() {
        setting_and_unsetting(256)
    }

    #[test]
    fn setting_and_unsetting_eight() {
        setting_and_unsetting(512)
    }

    #[test]
    fn setting_and_unsetting_sixteen() {
        setting_and_unsetting(1024)
    }

    #[cfg(test)]
    fn setting_and_unsetting(entries: usize) {
        let map = AtomicBitMap::new(entries);
        assert_eq!(map.capacity(), entries);

        // All bits should be empty.
        for data in &map.data {
            assert!(data.load(Ordering::Relaxed) == 0);
        }
        assert_eq!(map.next_set(), None);

        // Set all bits.
        for n in 0..entries {
            data.set(n);
        }
        for data in &map.data {
            assert!(data.load(Ordering::Relaxed) == usize::MAX);
        }

        // Unset all bits again.
        for n in 0..entries {
            assert_eq!(data.next_set(), Some(n));
        }
        // Now the map should be empty.
        assert_eq!(map.next_set(), None);
        for data in &map.data {
            assert!(data.load(Ordering::Relaxed) == 0);
        }

        // Test setting an index not in order.
        map.set(63);
        map.set(0);
        assert_eq!(map.next_set(), Some(0));
        assert_eq!(map.next_set(), Some(63));

        // Bitmap should be zeroed again.
        assert_eq!(map.next_set(), None);
        for data in &map.data {
            assert!(data.load(Ordering::Relaxed) == 0);
        }
    }
}
