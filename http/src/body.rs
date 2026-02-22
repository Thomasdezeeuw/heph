//! Module with HTTP body related types.
//!
//! See the [`Body`] trait.

use std::async_iter::AsyncIterator;
use std::future::Future;
use std::io;
use std::pin::pin;

use heph_rt::extract::Extract;
use heph_rt::fd::AsyncFd;
use heph_rt::io::Buf;
use heph_rt::util::next;

/// Last chunk of a body in a chunked response.
const LAST_CHUNK: &[u8] = b"0\r\n\r\n";

/// Trait that defines a HTTP body.
///
/// The trait can't be implemented outside of this crate and is implemented by
/// the following types:
///
/// * [`EmptyBody`]: no/empty body.
/// * [`OneshotBody`]: body consisting of a single chunk of bytes.
/// * [`StreamingBody`]: body that is streaming, with a known length.
/// * [`ChunkedBody`]: body that is streaming, with a *un*known length. This
///   uses HTTP chunked encoding to transfer the body.
///
/// Alternatively the [`AnyBody`] can be used to support all body types in one.
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

    use heph_rt::fd::AsyncFd;

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
            stream: &'stream mut AsyncFd,
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
        stream: &'stream mut AsyncFd,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'stream> {
        stream.send_all(http_head).extract()
    }
}

/// Body length and content is known in advance. Send in a single payload (i.e.
/// not chunked).
#[derive(Copy, Clone, Debug)]
pub struct OneshotBody<B> {
    bytes: B,
}

impl<B> OneshotBody<B>
where
    B: Buf,
{
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
        BodyLength::Known(self.bytes.len())
    }
}

impl<B: Buf> PrivateBody for OneshotBody<B> {
    type WriteFuture<'stream> = impl Future<Output = io::Result<Vec<u8>>> + 'stream;

    fn write_message<'stream>(
        self,
        stream: &'stream mut AsyncFd,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'stream> {
        let bufs = (http_head, self.bytes);
        async move {
            let (http_head, _) = stream.send_all_vectored(bufs).extract().await?;
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
    ///
    /// `length` must be the total body length in bytes.
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
        stream: &'stream mut AsyncFd,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'stream> {
        async move {
            let mut body = pin!(self.body);
            let http_head = stream.send_all(http_head).extract().await?;
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
        stream: &'stream mut AsyncFd,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'stream> {
        async move {
            let mut body = pin!(self.body);
            let http_head = stream.send_all(http_head).extract().await?;
            while let Some(chunk) = next(&mut body).await {
                _ = stream.send_all(chunk).await?;
            }
            _ = stream.send_all(LAST_CHUNK).await?;
            Ok(http_head)
        }
    }
}

/// Any kind of body.
///
/// Wraps all other variants in an enum.
///
/// When a variant is not used, you can use the never type (`!`) to mark the
/// variant as unusable (safing some memory and branches in the code).
///
/// # Examples
///
/// Using this never type (`!`) to remove unused variants.
///
/// ```
/// #![feature(never_type)]
/// use heph_http::body::AnyBody;
///
/// # type MyChunkedBody = ();
/// /// Using a static string for the oneshot body and `MyChunkedStream` for a
/// /// chunked stream.
/// type MyBody = AnyBody<&'static str, !, MyChunkedStream>;
/// ```
#[non_exhaustive]
#[derive(Debug)]
pub enum AnyBody<B = !, S = !, C = !> {
    /// [`EmptyBody`].
    Empty,
    /// [`OneshotBody`].
    Oneshot(OneshotBody<B>),
    /// [`StreamingBody`].
    Streaming(StreamingBody<S>),
    /// [`ChunkedBody`].
    Chunked(ChunkedBody<C>),
}

impl<B, S, C> AnyBody<B, S, C> {
    /// Create a new empty body, see [`EmptyBody`].
    pub const fn empty() -> AnyBody<B, S, C> {
        AnyBody::Empty
    }

    /// Create a new oneshot body, see [`OneshotBody`].
    pub const fn oneshot(buf: B) -> AnyBody<B, S, C>
    where
        B: Buf,
    {
        AnyBody::Oneshot(OneshotBody::new(buf))
    }

    /// Create a streaming body, see [`StreamingBody`].
    pub const fn streaming(length: usize, stream: S) -> AnyBody<B, S, C>
    where
        S: AsyncIterator + 'static,
        S::Item: Buf,
    {
        AnyBody::Streaming(StreamingBody::new(length, stream))
    }

    /// Create a chunked body, see [`ChunkedBody`].
    pub const fn chuncked<SB>(stream: C) -> AnyBody<B, S, C>
    where
        C: AsyncIterator + 'static,
        C::Item: Buf,
    {
        AnyBody::Chunked(ChunkedBody::new(stream))
    }
}

impl<B, S, C> From<EmptyBody> for AnyBody<B, S, C> {
    fn from(EmptyBody: EmptyBody) -> AnyBody<B, S, C> {
        AnyBody::empty()
    }
}

impl<B, S, C> From<OneshotBody<B>> for AnyBody<B, S, C>
where
    B: Buf,
{
    fn from(body: OneshotBody<B>) -> AnyBody<B, S, C> {
        AnyBody::Oneshot(body)
    }
}

impl<B, S, C> From<StreamingBody<S>> for AnyBody<B, S, C>
where
    S: AsyncIterator + 'static,
    S::Item: Buf,
{
    fn from(body: StreamingBody<S>) -> AnyBody<B, S, C> {
        AnyBody::Streaming(body)
    }
}

impl<B, S, C> From<ChunkedBody<C>> for AnyBody<B, S, C>
where
    C: AsyncIterator + 'static,
    C::Item: Buf,
{
    fn from(body: ChunkedBody<C>) -> AnyBody<B, S, C> {
        AnyBody::Chunked(body)
    }
}
