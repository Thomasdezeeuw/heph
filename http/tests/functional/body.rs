use std::async_iter::AsyncIterator;
use std::mem::replace;
use std::pin::Pin;
use std::task::{self, Poll};

use heph_http::body::{Body, BodyLength, ChunkedBody, EmptyBody, OneshotBody, StreamingBody};

use crate::{assert_send, assert_size, assert_sync};

const BODY0: &[u8] = b"";
const BODY1: &[u8] = b"Hello world!";

struct EmptyStream;

impl AsyncIterator for EmptyStream {
    type Item = &'static [u8];

    fn poll_next(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(None)
    }
}

struct SingleStream<'a>(&'a [u8]);

impl<'a> AsyncIterator for SingleStream<'a> {
    type Item = &'a [u8];

    fn poll_next(mut self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.0.is_empty() {
            Poll::Ready(Some(replace(&mut self.0, &[])))
        } else {
            Poll::Ready(None)
        }
    }
}

#[test]
fn size() {
    assert_size::<ChunkedBody<()>>(0);
    assert_size::<EmptyBody>(0);
    assert_size::<OneshotBody<&'static [u8]>>(16);
    assert_size::<StreamingBody<()>>(8);
}

#[test]
fn send() {
    assert_send::<ChunkedBody<()>>();
    assert_send::<EmptyBody>();
    assert_send::<OneshotBody<&'static [u8]>>();
    assert_send::<StreamingBody<()>>();
}

#[test]
fn sync() {
    assert_sync::<ChunkedBody<()>>();
    assert_sync::<EmptyBody>();
    assert_sync::<OneshotBody<&'static [u8]>>();
    assert_sync::<StreamingBody<()>>();
}

#[test]
fn empty_body() {
    assert_eq!(EmptyBody.length(), BodyLength::Known(0));
}

#[test]
fn oneshot_body() {
    assert_eq!(
        OneshotBody::new(BODY0).length(),
        BodyLength::Known(BODY0.len())
    );
    assert_eq!(
        OneshotBody::new(BODY1).length(),
        BodyLength::Known(BODY1.len())
    );
    assert_eq!(OneshotBody::new("abc").length(), BodyLength::Known(3));
}

#[test]
fn streaming_body() {
    assert_eq!(
        StreamingBody::new(0, EmptyStream).length(),
        BodyLength::Known(0)
    );
    assert_eq!(
        StreamingBody::new(0, SingleStream(BODY0)).length(),
        BodyLength::Known(0)
    );
    assert_eq!(
        // NOTE: wrong length!
        StreamingBody::new(0, SingleStream(BODY1)).length(),
        BodyLength::Known(0)
    );
}

#[test]
fn chunked_body() {
    assert_eq!(ChunkedBody::new(EmptyStream).length(), BodyLength::Chunked);
    assert_eq!(
        ChunkedBody::new(SingleStream(BODY0)).length(),
        BodyLength::Chunked
    );
    assert_eq!(
        // NOTE: wrong length!
        ChunkedBody::new(SingleStream(BODY1)).length(),
        BodyLength::Chunked
    );
}
