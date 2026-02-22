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
        type WriteFuture<'fd>: Future<Output = io::Result<Vec<u8>>> + 'fd;

        /// Write an HTTP message to `fd` using the `http_head` as head. Expects
        /// the `http_head` buffer to be returned.
        fn write_message<'fd>(
            self,
            fd: &'fd mut AsyncFd,
            http_head: Vec<u8>,
        ) -> Self::WriteFuture<'fd>;
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

impl<B: Buf> OneshotBody<B> {
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
    stream: S,
}

impl<S> StreamingBody<S>
where
    S: AsyncIterator + 'static,
    S::Item: Buf,
{
    /// Use a [`AsyncIterator`] as HTTP body with a known length.
    ///
    /// `length` must be the total body length in bytes.
    pub const fn new(length: usize, stream: S) -> StreamingBody<S> {
        StreamingBody { length, stream }
    }
}

impl<S> Body for StreamingBody<S>
where
    S: AsyncIterator + 'static,
    S::Item: Buf,
{
    fn length(&self) -> BodyLength {
        BodyLength::Known(self.length)
    }
}

impl<S> PrivateBody for StreamingBody<S>
where
    S: AsyncIterator + 'static,
    S::Item: Buf,
{
    type WriteFuture<'fd> = impl Future<Output = io::Result<Vec<u8>>> + 'fd;

    fn write_message<'fd>(
        self,
        fd: &'fd mut AsyncFd,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'fd> {
        async move {
            let mut stream = pin!(self.stream);
            let http_head = fd.send_all(http_head).extract().await?;
            while let Some(bytes) = next(&mut stream).await {
                _ = fd.send_all(bytes).await?;
            }
            Ok(http_head)
        }
    }
}

/// Streaming body with an unknown length. Send in multiple chunks.
#[derive(Debug)]
pub struct ChunkedBody<C> {
    chunks: C,
}

impl<C> ChunkedBody<C>
where
    C: AsyncIterator + 'static,
    C::Item: Buf,
{
    /// Use a [`AsyncIterator`] as HTTP body with a unknown length.
    ///
    /// If the total length of all chunks is known prefer to use
    /// [`StreamingBody`].
    pub const fn new(chunks: C) -> ChunkedBody<C> {
        ChunkedBody { chunks }
    }
}

impl<C> Body for ChunkedBody<C>
where
    C: AsyncIterator + 'static,
    C::Item: Buf,
{
    fn length(&self) -> BodyLength {
        BodyLength::Chunked
    }
}

impl<C> PrivateBody for ChunkedBody<C>
where
    C: AsyncIterator + 'static,
    C::Item: Buf,
{
    type WriteFuture<'fd> = impl Future<Output = io::Result<Vec<u8>>> + 'fd;

    fn write_message<'fd>(
        self,
        fd: &'fd mut AsyncFd,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'fd> {
        async move {
            let mut chunks = pin!(self.chunks);
            let http_head = fd.send_all(http_head).extract().await?;
            while let Some(chunk) = next(&mut chunks).await {
                _ = fd.send_all(chunk).await?;
            }
            _ = fd.send_all(LAST_CHUNK).await?;
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
/// # type MyChunkedStream = ();
/// /// Using a static string for the oneshot body and `MyChunkedStream` for a
/// /// chunked stream.
/// type MyBody = AnyBody<&'static str, !, MyChunkedStream>;
///
/// # const fn is_body<T: heph_http::body::Body>() {}
/// # const _TEST: () = is_body::<MyBody>();
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
    pub const fn chuncked<SB>(chunks: C) -> AnyBody<B, S, C>
    where
        C: AsyncIterator + 'static,
        C::Item: Buf,
    {
        AnyBody::Chunked(ChunkedBody::new(chunks))
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

impl<B, S, C> Body for AnyBody<B, S, C>
where
    B: Buf,
    S: AsyncIterator + 'static,
    S::Item: Buf,
    C: AsyncIterator + 'static,
    C::Item: Buf,
{
    fn length(&self) -> BodyLength {
        match self {
            AnyBody::Empty => EmptyBody.length(),
            AnyBody::Oneshot(body) => body.length(),
            AnyBody::Streaming(body) => body.length(),
            AnyBody::Chunked(body) => body.length(),
        }
    }
}

impl<B, S, C> PrivateBody for AnyBody<B, S, C>
where
    B: Buf,
    S: AsyncIterator + 'static,
    S::Item: Buf,
    C: AsyncIterator + 'static,
    C::Item: Buf,
{
    type WriteFuture<'fd> = impl Future<Output = io::Result<Vec<u8>>> + 'fd;

    fn write_message<'fd>(
        self,
        fd: &'fd mut AsyncFd,
        http_head: Vec<u8>,
    ) -> Self::WriteFuture<'fd> {
        async move {
            match self {
                AnyBody::Empty => EmptyBody.write_message(fd, http_head).await,
                AnyBody::Oneshot(body) => body.write_message(fd, http_head).await,
                AnyBody::Streaming(body) => body.write_message(fd, http_head).await,
                AnyBody::Chunked(body) => body.write_message(fd, http_head).await,
            }
        }
    }
}
