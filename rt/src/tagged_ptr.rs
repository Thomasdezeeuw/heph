//! Tagged pointers.
//!
//! See [`TaggedPointer`]

#![allow(dead_code)] // TODO: remove.

use std::num::NonZeroUsize;
use std::ptr::NonNull;

/// Pointer with a single 16 bit or two 8 bit tags.
#[repr(transparent)]
pub(crate) struct TaggedPointer<T> {
    // We use the 16 upper bits that are unused in 64 bit addresses for the tag.
    pointer: NonNull<T>,
}

const TAG_SHIFT: usize = (usize::BITS - u16::BITS) as usize;
const TAG_MASK: usize = !((u16::MAX as usize) << TAG_SHIFT);

impl<T> TaggedPointer<T> {
    /// Create a new `TaggedPointer` with a single 16 bit tag.
    pub(crate) fn new1(pointer: NonNull<T>, tag: u16) -> TaggedPointer<T> {
        TaggedPointer {
            pointer: pointer.map_addr(|ptr| ptr | (usize::from(tag) << TAG_SHIFT)),
        }
    }

    /// Create a new `TaggedPointer` with a two 8 bit tags.
    pub(crate) fn new2(pointer: NonNull<T>, tag1: u8, tag2: u8) -> TaggedPointer<T> {
        TaggedPointer::new1(pointer, u16::from_ne_bytes([tag1, tag2]))
    }

    /// Returns the underlying pointer.
    pub(crate) fn as_ptr(self) -> NonNull<T> {
        // SAFETY: because the pointer is not null when creating this type and
        // because we only remove the tag we're ensured that the pointer remains
        // not null.
        self.pointer
            .map_addr(|ptr| unsafe { NonZeroUsize::new_unchecked(ptr.get() & TAG_MASK) })
    }

    /// Returns the single 16 bit tag.
    pub(crate) fn tag1(self) -> u16 {
        (self.pointer.addr().get() >> TAG_SHIFT) as u16
    }

    /// Returns the two 8 bit tags.
    pub(crate) fn tag2(self) -> [u8; 2] {
        self.tag1().to_ne_bytes()
    }
}

impl<T> Copy for TaggedPointer<T> {}

impl<T> Clone for TaggedPointer<T> {
    fn clone(&self) -> TaggedPointer<T> {
        *self
    }
}

#[test]
fn tagged_pointer() {
    let alloc = Box::new(123usize);
    let original_ptr = Box::into_non_null(alloc);

    for tag in [0, 123, u16::from(u8::MAX) + 1, u16::MAX] {
        let tagged_ptr = TaggedPointer::new1(original_ptr, tag);
        let got_ptr = tagged_ptr.as_ptr();
        assert_eq!(got_ptr, original_ptr);
        let got_tag = tagged_ptr.tag1();
        assert_eq!(got_tag, tag);
    }

    for tags in [[0, 0], [123, 100], [0, u8::MAX], [u8::MAX, u8::MAX]] {
        let tagged_ptr = TaggedPointer::new2(original_ptr, tags[0], tags[1]);
        let got_ptr = tagged_ptr.as_ptr();
        assert_eq!(got_ptr, original_ptr);
        let got_tags = tagged_ptr.tag2();
        assert_eq!(got_tags[0], tags[0]);
        assert_eq!(got_tags[1], tags[1]);
    }

    drop(unsafe { Box::from_non_null(original_ptr) });
}
