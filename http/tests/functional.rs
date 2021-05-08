//! Functional tests.

use std::mem::size_of;

#[track_caller]
fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod from_header_value;
    mod header;
    mod method;
    mod status_code;
    mod version;
}
