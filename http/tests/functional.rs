//! Functional tests.

use std::mem::size_of;

#[track_caller]
fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod header;
    mod method;
    mod version;
}
