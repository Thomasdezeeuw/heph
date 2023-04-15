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
        for data in self.data.iter() {
            let value = data.load(Ordering::Relaxed);
            write!(f, "{value:0WIDTH$b}")?;
        }
        Ok(())
    }
}

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

    // Set all indices.
    for n in 0..entries {
        map.set(n);
    }

    // All bits should be set.
    for data in &map.data {
        assert!(data.load(Ordering::Relaxed) == usize::MAX);
    }

    // Unset all indices again.
    for n in 0..entries {
        assert_eq!(map.next_set(), Some(n));
    }
    // Bitmap should be zeroed.
    for data in &map.data {
        assert!(data.load(Ordering::Relaxed) == 0);
    }

    // Test unsetting an index not in order.
    map.set(63);
    map.set(0);
    assert!(matches!(map.next_set(), Some(i) if i == 0));
    assert!(matches!(map.next_set(), Some(i) if i == 63));

    // Next avaiable index should be 0 again.
    assert_eq!(map.next_set(), None);
}

#[test]
fn setting_and_unsetting_concurrent() {
    use std::sync::{Arc, Barrier};
    use std::thread;

    const N: usize = 4;
    const M: usize = 1024;

    let bitmap = Arc::new(AtomicBitMap::new(N * M));

    for n in 0..N * M {
        bitmap.set(n);
    }

    let barrier = Arc::new(Barrier::new(N + 1));
    let handles = (0..N)
        .map(|i| {
            let bitmap = bitmap.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let mut indices = Vec::with_capacity(M);
                _ = barrier.wait();

                if i % 2 == 0 {
                    for _ in 0..M {
                        let idx = bitmap.next_set().expect("failed to get index");
                        indices.push(idx);
                    }

                    for idx in indices {
                        bitmap.set(idx);
                    }
                } else {
                    for _ in 0..M {
                        let idx = bitmap.next_set().expect("failed to get index");
                        bitmap.set(idx);
                    }
                }
            })
        })
        .collect::<Vec<_>>();

    _ = barrier.wait();
    handles
        .into_iter()
        .map(|handle| handle.join())
        .collect::<thread::Result<()>>()
        .unwrap();
}
