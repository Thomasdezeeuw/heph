//! Functional tests.

#![feature(async_stream, never_type, once_cell)]

use std::mem::size_of;

#[track_caller]
fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

fn assert_send<T: Send>() {}

fn assert_sync<T: Sync>() {}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    // TODO.
    //mod udp;
    //mod tcp;
}
