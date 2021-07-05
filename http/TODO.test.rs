// TODO: also tests all body types.
//
// FIXME: currently `Connection::respond` is `!Send` and `!Sync`, fix that.

fn assert_send<T: Send>() {}

fn assert_sync<T: Sync>() {}

fn assert_send_ref<T: Send>(_: &T) {}

fn assert_sync_ref<T: Sync>(_: &T) {}

/* TODO: add tests.

let fut = connection.next_request();
assert_send_ref(&fut);
assert_sync_ref(&fut);

let body = OneshotBody::new(body.as_bytes());
let fut = connection.respond(code, &headers, body);
assert_send_ref(&fut);
assert_sync_ref(&fut);
fut.await?;

let fut = connection.respond(code, &headers, http::body::EmptyBody);
assert_send_ref(&fut);
assert_sync_ref(&fut);
fut.await?;

let f = std::fs::File::open("abc").unwrap();
let body = http::body::FileBody::new(&f, 0, std::num::NonZeroUsize::new(123).unwrap());
let fut = connection.respond(code, &headers, body);
assert_send_ref(&fut);
assert_sync_ref(&fut);
fut.await?;

let bytes = b"hello world";
let body = http::body::StreamingBody::new(bytes.len(), ByteStream::new(bytes));
let fut = connection.respond(code, &headers, body);
assert_send_ref(&fut);
assert_sync_ref(&fut);

struct ByteStream<'b> {
    bytes: Option<&'b [u8]>,
}

impl<'b> ByteStream<'b> {
    const fn new(bytes: &'b [u8]) -> ByteStream<'b> {
        ByteStream { bytes: Some(bytes) }
    }
}

impl<'b> Stream for ByteStream<'b> {
    type Item = io::Result<&'b [u8]>;

    fn poll_next(mut self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.bytes.take().map(Ok))
    }
}
*/

// For Body:

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io;
    use std::marker::PhantomData;
    use std::pin::Pin;
    use std::stream::Stream;
    use std::task::{self, Poll};

    use super::{
        ChunkedBody, EmptyBody, FileBody, OneshotBody, SendAll, SendFileBody, SendOneshotBody,
        SendStreamingBody, StreamingBody,
    };

    struct BodyStream<'b> {
        _body_lifetime: PhantomData<&'b [u8]>,
    }

    impl<'b> Stream for BodyStream<'b> {
        type Item = io::Result<&'b [u8]>;

        fn poll_next(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(None)
        }
    }

    fn assert_send<T: Send>() {}

    fn assert_sync<T: Sync>() {}

    #[test]
    fn empty_body_is_send_sync() {
        assert_send::<EmptyBody>();
        assert_sync::<EmptyBody>();
        // This is `heph::net::tcp::stream::SendAll`.
        assert_send::<SendAll>();
        assert_sync::<SendAll>();
    }

    #[test]
    fn oneshot_body_is_send_sync() {
        assert_send::<OneshotBody>();
        assert_sync::<OneshotBody>();
        assert_send::<SendOneshotBody>();
        assert_sync::<SendOneshotBody>();
    }

    #[test]
    fn streaming_body_is_send_sync() {
        assert_send::<StreamingBody<BodyStream>>();
        assert_sync::<StreamingBody<BodyStream>>();
        assert_send::<SendStreamingBody<BodyStream>>();
        assert_sync::<SendStreamingBody<BodyStream>>();
    }

    #[test]
    fn chunked_body_is_send_sync() {
        assert_send::<ChunkedBody<BodyStream>>();
        assert_sync::<ChunkedBody<BodyStream>>();
        // TODO: add `SendChunkedBody`.
    }

    #[test]
    fn file_body_is_send_sync() {
        assert_send::<FileBody<File>>();
        assert_sync::<FileBody<File>>();
        assert_send::<SendFileBody<File>>();
        assert_sync::<SendFileBody<File>>();
    }
}
