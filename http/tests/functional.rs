//! Functional tests.

#![feature(async_stream, never_type, once_cell)]

use std::mem::size_of;

#[track_caller]
fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod client;
    mod from_header_value;
    mod head;
    mod header;
    mod method;
    mod server;
    mod status_code;
    mod version;
}
