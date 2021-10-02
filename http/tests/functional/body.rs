/*
use std::fs::File;
use std::num::NonZeroUsize;

use heph_http::body::*;

use crate::{assert_send, assert_size, assert_sync, EmptyStream, SingleStream};

const BODY0: &[u8] = b"";
const BODY1: &[u8] = b"Hello world!";

#[test]
fn size() {
    assert_size::<ChunkedBody<()>>(0);
    assert_size::<EmptyBody>(0);
    assert_size::<FileBody<File>>(24);
    assert_size::<OneshotBody>(16);
    assert_size::<StreamingBody<()>>(8);
}

#[test]
fn send() {
    assert_send::<ChunkedBody<()>>();
    assert_send::<EmptyBody>();
    assert_send::<FileBody<File>>();
    assert_send::<OneshotBody>();
    assert_send::<StreamingBody<()>>();
}

#[test]
fn sync() {
    assert_sync::<ChunkedBody<()>>();
    assert_sync::<EmptyBody>();
    assert_sync::<FileBody<File>>();
    assert_sync::<OneshotBody>();
    assert_sync::<StreamingBody<()>>();
}

#[test]
fn empty_body() {
    assert_eq!(EmptyBody.length(), BodyLength::Known(0));
}

#[test]
fn oneshot_bytes() {
    let body = OneshotBody::new(BODY1);
    assert_eq!(body.bytes(), BODY1);
}

#[test]
fn oneshot_cmp_bytes() {
    let body = OneshotBody::new(BODY1);
    assert_eq!(body, BODY1);
}

#[test]
fn oneshot_cmp_string() {
    let body = OneshotBody::new(BODY1);
    assert_eq!(body, "Hello world!");
}

#[test]
fn oneshot_body() {
    assert_eq!(
        OneshotBody::new(BODY0).length(),
        BodyLength::Known(BODY0.len())
    );
    assert_eq!(
        OneshotBody::from(BODY1).length(),
        BodyLength::Known(BODY1.len())
    );
    assert_eq!(OneshotBody::from("abc").length(), BodyLength::Known(3));
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
fn file_body() {
    let file = File::open("Cargo.toml").unwrap();
    assert_eq!(
        FileBody::new(&file, 0, NonZeroUsize::new(10).unwrap()).length(),
        BodyLength::Known(10)
    );
    assert_eq!(
        FileBody::new(&file, 5, NonZeroUsize::new(10).unwrap()).length(),
        BodyLength::Known(5)
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
*/
