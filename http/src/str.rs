//! Module with [`Str`] types.
//!
//! The [`Str`] type supports borrowed and owned (heap allocated) strings in the
//! same type.

use std::marker::PhantomData;
use std::panic::{RefUnwindSafe, UnwindSafe};
use std::{fmt, slice, str};

/// Marker on [`Str::length`] to indicate the string is heap allocated.
const MARK_OWNED: usize = 1 << (usize::BITS - 1);

/// `Str` is combination of `str` and `String`.
///
/// It supports both borrowed and owned (heap allocated) strings in the same
/// type.
pub(crate) struct Str<'a> {
    /// Pointer must always be valid and point to atleast `length` bytes that
    /// are valid UTF-8.
    ptr: *const u8,
    /// Number of bytes that are valid UTF-8.
    ///
    /// This is marked, use [`Str::len()`] to ge the correct length.
    length: usize,
    _phantom: PhantomData<&'a str>,
}

impl Str<'static> {
    /// Create an owned `Str` from `string`.
    ///
    /// # Notes
    ///
    /// This "leaks" the unused capacity of `string`.
    pub(crate) fn from_string(string: String) -> Str<'static> {
        let bytes = string.into_bytes().leak();
        Str {
            ptr: bytes.as_ptr(),
            length: bytes.len() | MARK_OWNED,
            _phantom: PhantomData,
        }
    }
}

impl<'a> Str<'a> {
    /// Create a borrowed `Str` from `string`.
    pub(crate) const fn from_str(string: &'a str) -> Str<'a> {
        Str {
            ptr: string.as_ptr(),
            length: string.len(),
            _phantom: PhantomData,
        }
    }

    /// Borrow the string for a shorter lifetime.
    pub(crate) const fn borrow<'b>(&self) -> Str<'b>
    where
        'a: 'b,
    {
        Str {
            ptr: self.ptr,
            length: self.len(),
            _phantom: PhantomData,
        }
    }

    /// Returns the `Str` as `str`.
    const fn as_str(&self) -> &'a str {
        // SAFETY: safe based on the docs of `ptr` and `length`.
        unsafe { str::from_utf8_unchecked(slice::from_raw_parts(self.ptr, self.len())) }
    }

    /// Returns the length of the string.
    const fn len(&self) -> usize {
        self.length & !MARK_OWNED
    }

    /// Returns `true` if `self` is heap allocated.
    pub(crate) const fn is_heap_allocated(&self) -> bool {
        self.length & MARK_OWNED != 0
    }
}

impl<'a> AsRef<str> for Str<'a> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<'a> Clone for Str<'a> {
    fn clone(&self) -> Str<'a> {
        Str::from_str(self.as_str())
    }
}

impl<'a> Eq for Str<'a> {}

impl<'a> PartialEq<Str<'a>> for Str<'a> {
    fn eq(&self, other: &Str<'a>) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl<'a> PartialEq<str> for Str<'a> {
    fn eq(&self, other: &str) -> bool {
        self.as_ref() == other
    }
}

impl<'a> PartialEq<&'_ str> for Str<'a> {
    fn eq(&self, other: &&str) -> bool {
        self.eq(*other)
    }
}

// SAFETY: Since this is just a `String`/`&str` it's safe to send accross
// threads.
unsafe impl<'a> Send for Str<'a> {}
unsafe impl<'a> Sync for Str<'a> {}

impl<'a> UnwindSafe for Str<'a> {}
impl<'a> RefUnwindSafe for Str<'a> {}

impl<'a> Drop for Str<'a> {
    fn drop(&mut self) {
        if self.is_heap_allocated() {
            let len = self.len();
            unsafe { drop(Vec::<u8>::from_raw_parts(self.ptr.cast_mut(), len, len)) }
        }
    }
}

impl<'a> fmt::Debug for Str<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}
