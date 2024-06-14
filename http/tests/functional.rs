//! Functional tests.

#![feature(async_iterator, never_type)]

#[track_caller]
fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

fn assert_send<T: Send>() {}

fn assert_sync<T: Sync>() {}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod body;
    mod client;
    mod from_header_value;
    mod header;
    mod message;
    mod method;
    mod route;
    mod server;
    mod status_code;
    mod transform;
    mod version;
}
