//! Functional tests.

#![feature(async_stream, never_type, once_cell)]

use std::io;
use std::mem::replace;
use std::mem::size_of;
use std::pin::Pin;
use std::stream::Stream;
use std::task::{self, Poll};

#[track_caller]
fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

fn assert_send<T: Send>() {}
fn assert_send_ref<T: Send>(_: &T) {}

fn assert_sync<T: Sync>() {}
fn assert_sync_ref<T: Sync>(_: &T) {}

struct EmptyStream;

impl Stream for EmptyStream {
    type Item = io::Result<&'static [u8]>;

    fn poll_next(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(None)
    }
}

struct SingleStream<'a>(&'a [u8]);

impl<'a> Stream for SingleStream<'a> {
    type Item = io::Result<&'a [u8]>;

    fn poll_next(mut self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.0.is_empty() {
            Poll::Ready(Some(Ok(replace(&mut self.0, &[]))))
        } else {
            Poll::Ready(None)
        }
    }
}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod body;
    mod client;
    mod from_header_value;
    mod header;
    mod message;
    mod method;
    mod server;
    mod status_code;
    mod version;
}
