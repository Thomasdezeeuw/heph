//! Module with HTTP body related types.
//!
//! See the [`Body`] trait.

use std::async_iter::AsyncIterator;
use std::future::Future;
use std::io;
use std::pin::pin;

use heph_rt::io::Buf;
use heph_rt::net::TcpStream;
use heph_rt::util::next;

/// Last chunk of a body in a chunked response.
const LAST_CHUNK: &[u8] = b"0\r\n\r\n";

/// Trait that defines a HTTP body.
///
/// The trait can't be implemented outside of this create and is implemented by
/// the following types:
///
/// * [`EmptyBody`]: no/empty body.
/// * [`OneshotBody`]: body consisting of a single chunk of bytes.
/// * [`StreamingBody`]: body that is streaming, with a known length.
/// * [`ChunkedBody`]: body that is streaming, with a *un*known length. This
///   uses HTTP chunked encoding to transfer the body.
pub trait Body: PrivateBody {
    /// Length of the body, or the body will be chunked.
    fn length(&self) -> BodyLength;
}

/// Length of a body.
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BodyLength {
    /// Body length is known.
    Known(usize),
    /// Body length is unknown and the body will be transfered using chunked
    /// encoding.
    Chunked,
}

mod private {
    use std::future::Future;
    use std::io;

    use heph_rt::net::TcpStream;

    /// Private extention of [`Body`].
    ///
    /// [`Body`]: super::Body
    pub trait PrivateBody {
        /// [`Future`] behind [`PrivateBody::write_message`].
        type WriteFuture<'stream>: Future<Output = io::Result<Vec<u8>>> + 'stream;

        /// Write an HTTP message to `stream` using the `http_head` as head.
        /// Expects the `http_head` buffer to be returned.
        fn write_message<'stream>(
            self,
            stream: &'stream mut TcpStream,
            http_head: Vec<u8>,
        ) -> Self::WriteFuture<'stream>;
    }
}

pub(crate) use private::PrivateBody;

/// An empty body.
#[derive(Copy, Clone, Debug)]
pub struct EmptyBody;

impl Body for EmptyBody {
    fn length(&self) -> BodyLength {
        BodyLength::Known(0)
    }
}

impl PrivateBody for EmptyBody {
    type WriteFuture<'stream> = impl Future<Output = io::Result<Vec<u8>>> + 'stream;

    fn write_message<'stream>(
        self,
        stream: &'stream mut TcpStream,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'stream> {
        stream.send_all(http_head)
    }
}

/// Body length and content is known in advance. Send in a single payload (i.e.
/// not chunked).
#[derive(Copy, Clone, Debug)]
pub struct OneshotBody<B> {
    bytes: B,
}

impl<B> OneshotBody<B> {
    /// Create a new one-shot body.
    pub const fn new(body: B) -> OneshotBody<B> {
        OneshotBody { bytes: body }
    }

    /// Returns the underlying buffer.
    pub fn into_inner(self) -> B {
        self.bytes
    }
}

impl<B: Buf> Body for OneshotBody<B> {
    fn length(&self) -> BodyLength {
        // SAFETY: only using the length, nothing unsafe about that.
        BodyLength::Known(unsafe { self.bytes.parts().1 })
    }
}

impl<B: Buf> PrivateBody for OneshotBody<B> {
    type WriteFuture<'stream> = impl Future<Output = io::Result<Vec<u8>>> + 'stream;

    fn write_message<'stream>(
        self,
        stream: &'stream mut TcpStream,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'stream> {
        let bufs = (http_head, self.bytes);
        async move {
            let (http_head, _) = stream.send_vectored_all(bufs).await?;
            Ok(http_head)
        }
    }
}

/// Streaming body with a known length. Send in a single payload (i.e. not
/// chunked).
#[derive(Debug)]
pub struct StreamingBody<S> {
    length: usize,
    body: S,
}

impl<S, B> StreamingBody<S>
where
    S: AsyncIterator<Item = B> + 'static,
    B: Buf,
{
    /// Use a [`AsyncIterator`] as HTTP body with a known length.
    pub const fn new(length: usize, stream: S) -> StreamingBody<S> {
        StreamingBody {
            length,
            body: stream,
        }
    }
}

impl<S, B> Body for StreamingBody<S>
where
    S: AsyncIterator<Item = B> + 'static,
    B: Buf,
{
    fn length(&self) -> BodyLength {
        BodyLength::Known(self.length)
    }
}

impl<S, B> PrivateBody for StreamingBody<S>
where
    S: AsyncIterator<Item = B> + 'static,
    B: Buf,
{
    type WriteFuture<'stream> = impl Future<Output = io::Result<Vec<u8>>> + 'stream;

    fn write_message<'stream>(
        self,
        stream: &'stream mut TcpStream,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'stream> {
        async move {
            let mut body = pin!(self.body);
            let http_head = stream.send_all(http_head).await?;
            while let Some(chunk) = next(&mut body).await {
                _ = stream.send_all(chunk).await?;
            }
            Ok(http_head)
        }
    }
}

/// Streaming body with an unknown length. Send in multiple chunks.
#[derive(Debug)]
pub struct ChunkedBody<S> {
    body: S,
}

impl<S, B> ChunkedBody<S>
where
    S: AsyncIterator<Item = B> + 'static,
    B: Buf,
{
    /// Use a [`AsyncIterator`] as HTTP body with a unknown length.
    ///
    /// If the total length of `stream` is known prefer to use
    /// [`StreamingBody`].
    pub const fn new(stream: S) -> ChunkedBody<S> {
        ChunkedBody { body: stream }
    }
}

impl<S, B> Body for ChunkedBody<S>
where
    S: AsyncIterator<Item = B> + 'static,
    B: Buf,
{
    fn length(&self) -> BodyLength {
        BodyLength::Chunked
    }
}

impl<S, B> PrivateBody for ChunkedBody<S>
where
    S: AsyncIterator<Item = B> + 'static,
    B: Buf,
{
    type WriteFuture<'stream> = impl Future<Output = io::Result<Vec<u8>>> + 'stream;

    fn write_message<'stream>(
        self,
        stream: &'stream mut TcpStream,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'stream> {
        async move {
            let mut body = pin!(self.body);
            let http_head = stream.send_all(http_head).await?;
            while let Some(chunk) = next(&mut body).await {
                _ = stream.send_all(chunk).await?;
            }
            _ = stream.send_all(LAST_CHUNK).await?;
            Ok(http_head)
        }
    }
}
