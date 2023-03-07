//! Module with HTTP body related types.
//!
//! See the [`Body`] trait.

use std::async_iter::AsyncIterator;
use std::io::{self, IoSlice};
use std::marker::PhantomData;
use std::num::NonZeroUsize;

use heph_rt::net::tcp::stream::{FileSend, SendAll, TcpStream};

/// Trait that defines a HTTP body.
///
/// The trait can't be implemented outside of this create and is implemented by
/// the following types:
///
/// * [`EmptyBody`]: no/empty body.
/// * [`OneshotBody`]: body consisting of a single slice of bytes (`&[u8]`).
/// * [`StreamingBody`]: body that is streaming, with a known length.
/// * [`ChunkedBody`]: body that is streaming, with a *un*known length. This
///   uses HTTP chunked encoding to transfer the body.
/// * [`FileBody`]: uses a file as body, sending it's content using the
///   `sendfile(2)` system call.
pub trait Body<'a>: PrivateBody<'a> {
    /// Length of the body, or the body will be chunked.
    fn length(&self) -> BodyLength;
}

/// Length of a body.
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BodyLength {
    /// Body length is known.
    Known(usize),
    /// Body length is unknown and the body will be transferred using chunked
    /// encoding.
    Chunked,
}

mod private {
    use std::async_iter::AsyncIterator;
    use std::future::Future;
    use std::io::{self, IoSlice};
    use std::num::NonZeroUsize;
    use std::pin::Pin;
    use std::task::{self, Poll};

    use heph_rt::net::tcp::stream::FileSend;
    use heph_rt::net::TcpStream;

    const LAST_CHUNK: &[u8] = b"0\r\n\r\n";

    /// Private extension of [`Body`].
    ///
    /// [`Body`]: super::Body
    pub trait PrivateBody<'body> {
        /// [`Future`] behind [`PrivateBody::write_message`].
        type WriteMessage<'stream, 'head>: Future<Output = io::Result<()>>
        where
            Self: 'body;

        /// Write a HTTP message to `stream`.
        ///
        /// The `http_head` buffer contains the HTTP header (i.e. request/status
        /// line and all headers), this must still be written to the `stream`
        /// also.
        fn write_message<'stream, 'head>(
            self,
            stream: &'stream mut TcpStream,
            http_head: &'head [u8],
        ) -> Self::WriteMessage<'stream, 'head>
        where
            Self: 'body;
    }

    /// See [`super::OneshotBody`].
    #[derive(Debug)]
    pub struct SendOneshotBody<'s, 'b> {
        pub(super) stream: &'s mut TcpStream,
        // HTTP head and body.
        pub(super) bufs: [IoSlice<'b>; 2],
    }

    impl<'s, 'b> Future for SendOneshotBody<'s, 'b> {
        type Output = io::Result<()>;

        fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
            let SendOneshotBody { stream, bufs } = Pin::into_inner(self);
            loop {
                match stream.try_send_vectored(bufs) {
                    Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                    Ok(n) => {
                        let head_len = bufs[0].len();
                        let body_len = bufs[1].len();
                        if n >= head_len + body_len {
                            // Written everything.
                            return Poll::Ready(Ok(()));
                        } else if n <= head_len {
                            // Only written part of the head, advance the head
                            // buffer.
                            bufs[0].advance(n);
                        } else {
                            // Written entire head.
                            bufs[0] = IoSlice::new(&[]);
                            bufs[1].advance(n - head_len);
                        }
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Poll::Pending
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }
        }
    }

    /// See [`super::StreamingBody`].
    #[derive(Debug)]
    pub struct SendStreamingBody<'s, 'h, 'b, B> {
        pub(super) stream: &'s mut TcpStream,
        pub(super) head: &'h [u8],
        /// Bytes left to write from `body`, not counting the HTTP head.
        pub(super) left: usize,
        pub(super) body: B,
        /// Slice of bytes from `body`.
        pub(super) body_bytes: Option<&'b [u8]>,
    }

    impl<'s, 'h, 'b, B> Future for SendStreamingBody<'s, 'h, 'b, B>
    where
        B: AsyncIterator<Item = io::Result<&'b [u8]>>,
    {
        type Output = io::Result<()>;

        fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
            // SAFETY: not moving `body: B`, ensuring it's still pinned.
            #[rustfmt::skip]
            let SendStreamingBody { stream, head, left, body, body_bytes } = unsafe { Pin::into_inner_unchecked(self) };
            let mut body = unsafe { Pin::new_unchecked(body) };

            // Send the HTTP head first.
            // TODO: try to use vectored I/O on first call.
            while !head.is_empty() {
                match stream.try_send(*head) {
                    Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                    Ok(n) => *head = &head[n..],
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Poll::Pending
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }

            while *left != 0 {
                // We have bytes we need to send.
                if let Some(bytes) = body_bytes.as_mut() {
                    // TODO: check `bytes.len()` <= `left`.
                    match stream.try_send(*bytes) {
                        Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                        Ok(n) => {
                            *left -= n;
                            if n >= bytes.len() {
                                *body_bytes = None;
                            } else {
                                *bytes = &bytes[n..];
                                continue;
                            }
                        }
                        Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                            return Poll::Pending
                        }
                        Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                        Err(err) => return Poll::Ready(Err(err)),
                    }
                }

                // Read some bytes from the `body` stream.
                match body.as_mut().poll_next(ctx) {
                    Poll::Ready(Some(Ok(bytes))) => *body_bytes = Some(bytes),
                    Poll::Ready(Some(Err(err))) => return Poll::Ready(Err(err)),
                    Poll::Ready(None) => {
                        // NOTE: this shouldn't happend.
                        debug_assert!(*left == 0, "short body provided to `StreamingBody`");
                        return Poll::Ready(Ok(()));
                    }
                    Poll::Pending => return Poll::Pending,
                }
            }

            Poll::Ready(Ok(()))
        }
    }

    /// See [`super::ChunkedBody`].
    #[derive(Debug)]
    pub struct SendChunkedBody<'s, 'h, 'b, B> {
        pub(super) stream: &'s mut TcpStream,
        pub(super) head: &'h [u8],
        pub(super) body: B,
        /// Slice of bytes from `body`.
        pub(super) body_bytes: Option<&'b [u8]>,
        pub(super) written_chunk_size: bool,
    }

    impl<'s, 'h, 'b, B> Future for SendChunkedBody<'s, 'h, 'b, B>
    where
        B: AsyncIterator<Item = io::Result<&'b [u8]>>,
    {
        type Output = io::Result<()>;

        fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
            // SAFETY: not moving `body: B`, ensuring it's still pinned.
            #[rustfmt::skip]
            let SendChunkedBody { stream, head, body, body_bytes, written_chunk_size } = unsafe { Pin::into_inner_unchecked(self) };
            let mut body = unsafe { Pin::new_unchecked(body) };

            // Send the HTTP head first.
            // TODO: try to use vectored I/O on first call.
            while !head.is_empty() {
                match stream.try_send(*head) {
                    Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                    Ok(n) => *head = &head[n..],
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Poll::Pending
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }

            loop {
                // We have bytes we need to send.
                if let Some(bytes) = body_bytes.as_mut() {
                    let mut size_buf = itoa::Buffer::new();
                    let (b1, b2) = if *written_chunk_size {
                        // Already written the chunk size.
                        ("", "")
                    } else {
                        (size_buf.format(bytes.len()), "\r\n")
                    };

                    let mut bufs = [
                        // Chunk size.
                        IoSlice::new(b1.as_bytes()),
                        IoSlice::new(b2.as_bytes()),
                        IoSlice::new(bytes),   // User's bytes.
                        IoSlice::new(b"\r\n"), // End of chunk.
                    ];
                    loop {
                        match stream.try_send_vectored(&bufs) {
                            Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                            Ok(mut n) => {
                                // FIXME: deal with `n` < `b1.len() + b2.len()`.
                                n -= b1.len() + b2.len();
                                if n >= bytes.len() {
                                    *body_bytes = None;
                                    break;
                                }
                                *bytes = &bytes[n..];
                                bufs[2] = IoSlice::new(bytes);
                                continue;
                            }
                            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                                return Poll::Pending
                            }
                            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                            Err(err) => return Poll::Ready(Err(err)),
                        }
                    }
                }

                // Read some bytes from the `body` stream.
                match body.as_mut().poll_next(ctx) {
                    Poll::Ready(Some(Ok(bytes))) => {
                        *body_bytes = Some(bytes);
                        *written_chunk_size = false;
                    }
                    Poll::Ready(Some(Err(err))) => return Poll::Ready(Err(err)),
                    Poll::Ready(None) => loop {
                        match stream.try_send(LAST_CHUNK) {
                            // FIXME: properly deal with small write here.
                            Ok(n) if n < LAST_CHUNK.len() => {
                                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()))
                            }
                            Ok(_) => return Poll::Ready(Ok(())),
                            // FIXME: properly deal with this error; can't poll
                            // anymore.
                            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                                return Poll::Pending
                            }
                            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                            Err(err) => return Poll::Ready(Err(err)),
                        }
                    },
                    Poll::Pending => return Poll::Pending,
                }
            }
        }
    }

    /// See [`super::FileBody`].
    #[derive(Debug)]
    pub struct SendFileBody<'s, 'h, 'f, F> {
        pub(super) stream: &'s mut TcpStream,
        pub(super) head: &'h [u8],
        pub(super) file: &'f F,
        pub(super) offset: usize,
        pub(super) end: NonZeroUsize,
    }

    impl<'s, 'h, 'f, F> Future for SendFileBody<'s, 'h, 'f, F>
    where
        F: FileSend,
    {
        type Output = io::Result<()>;

        fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
            #[rustfmt::skip]
            let SendFileBody { stream, head, file, offset, end } = Pin::into_inner(self);

            // Send the HTTP head first.
            while !head.is_empty() {
                match stream.try_send(head) {
                    Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                    Ok(n) => *head = &head[n..],
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Poll::Pending
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }

            while end.get() > *offset {
                let length = NonZeroUsize::new(end.get() - *offset);
                match stream.try_send_file(*file, *offset, length) {
                    // All bytes were send.
                    Ok(0) => return Poll::Ready(Ok(())),
                    Ok(n) => *offset += n,
                    Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                        return Poll::Pending
                    }
                    Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) => return Poll::Ready(Err(err)),
                }
            }

            Poll::Ready(Ok(()))
        }
    }
}

pub(crate) use private::{PrivateBody, SendChunkedBody, SendStreamingBody};
use private::{SendFileBody, SendOneshotBody};

/// An empty body.
#[derive(Copy, Clone, Debug)]
pub struct EmptyBody;

impl<'b> Body<'b> for EmptyBody {
    fn length(&self) -> BodyLength {
        BodyLength::Known(0)
    }
}

impl<'b> PrivateBody<'b> for EmptyBody {
    type WriteMessage<'s, 'h> = SendAll<'s, 'h> where Self: 'b;

    fn write_message<'s, 'h>(
        self,
        stream: &'s mut TcpStream,
        http_head: &'h [u8],
    ) -> Self::WriteMessage<'s, 'h>
    where
        Self: 'b,
    {
        // Just need to write the HTTP head as we don't have a body.
        stream.send_all(http_head)
    }
}

/// Body length and content is known in advance. Send in a single payload (i.e.
/// not chunked).
#[derive(Debug, Clone)]
pub struct OneshotBody<'b> {
    bytes: &'b [u8],
}

impl<'b> OneshotBody<'b> {
    /// Create a new one-shot body.
    pub const fn new(body: &'b [u8]) -> OneshotBody<'b> {
        OneshotBody { bytes: body }
    }

    /// Returns the bytes that make up the body.
    pub const fn bytes(&'b self) -> &'b [u8] {
        self.bytes
    }
}

impl<'b> Body<'b> for OneshotBody<'b> {
    fn length(&self) -> BodyLength {
        BodyLength::Known(self.bytes.len())
    }
}

impl<'b> PrivateBody<'b> for OneshotBody<'b> {
    type WriteMessage<'s, 'h> = SendOneshotBody<'s, 'h>
    where Self: 'b;

    fn write_message<'s, 'h>(
        self,
        stream: &'s mut TcpStream,
        http_head: &'h [u8],
    ) -> Self::WriteMessage<'s, 'h>
    where
        Self: 'b,
    {
        let head = IoSlice::new(http_head);
        let body = IoSlice::new(self.bytes);
        SendOneshotBody {
            stream,
            bufs: [head, body],
        }
    }
}

impl<'b> From<&'b [u8]> for OneshotBody<'b> {
    fn from(body: &'b [u8]) -> Self {
        OneshotBody::new(body)
    }
}

impl<'b> From<&'b str> for OneshotBody<'b> {
    fn from(body: &'b str) -> Self {
        OneshotBody::new(body.as_bytes())
    }
}

impl<'b> PartialEq<[u8]> for OneshotBody<'b> {
    fn eq(&self, other: &[u8]) -> bool {
        self.bytes.eq(other)
    }
}

impl<'b> PartialEq<&[u8]> for OneshotBody<'b> {
    fn eq(&self, other: &&[u8]) -> bool {
        self.bytes.eq(*other)
    }
}

impl<'b> PartialEq<str> for OneshotBody<'b> {
    fn eq(&self, other: &str) -> bool {
        self.bytes.eq(other.as_bytes())
    }
}

impl<'b> PartialEq<&str> for OneshotBody<'b> {
    fn eq(&self, other: &&str) -> bool {
        self.bytes.eq(other.as_bytes())
    }
}

/// Streaming body with a known length. Send in a single payload (i.e. not
/// chunked).
#[derive(Debug)]
pub struct StreamingBody<'b, B> {
    length: usize,
    body: B,
    _body_lifetime: PhantomData<&'b [u8]>,
}

impl<'b, B> StreamingBody<'b, B>
where
    B: AsyncIterator<Item = io::Result<&'b [u8]>>,
{
    /// Use a [`AsyncIterator`] as HTTP body with a known length.
    pub const fn new(length: usize, stream: B) -> StreamingBody<'b, B> {
        StreamingBody {
            length,
            body: stream,
            _body_lifetime: PhantomData,
        }
    }
}

impl<'b, B> Body<'b> for StreamingBody<'b, B>
where
    B: AsyncIterator<Item = io::Result<&'b [u8]>>,
{
    fn length(&self) -> BodyLength {
        BodyLength::Known(self.length)
    }
}

impl<'b, B> PrivateBody<'b> for StreamingBody<'b, B>
where
    B: AsyncIterator<Item = io::Result<&'b [u8]>>,
{
    type WriteMessage<'s, 'h> = SendStreamingBody<'s, 'h, 'b, B>
        where Self: 'b;

    fn write_message<'s, 'h>(
        self,
        stream: &'s mut TcpStream,
        head: &'h [u8],
    ) -> Self::WriteMessage<'s, 'h>
    where
        Self: 'b,
    {
        SendStreamingBody {
            stream,
            body: self.body,
            head,
            left: self.length,
            body_bytes: None,
        }
    }
}

/// Streaming body with an unknown length. Send in multiple chunks.
#[derive(Debug)]
pub struct ChunkedBody<'b, B> {
    body: B,
    _body_lifetime: PhantomData<&'b [u8]>,
}

impl<'b, B> ChunkedBody<'b, B>
where
    B: AsyncIterator<Item = io::Result<&'b [u8]>>,
{
    /// Use a [`AsyncIterator`] as HTTP body with a unknown length.
    ///
    /// If the total length of `stream` is known prefer to use
    /// [`StreamingBody`].
    pub const fn new(stream: B) -> ChunkedBody<'b, B> {
        ChunkedBody {
            body: stream,
            _body_lifetime: PhantomData,
        }
    }
}

impl<'b, B> Body<'b> for ChunkedBody<'b, B>
where
    B: AsyncIterator<Item = io::Result<&'b [u8]>>,
{
    fn length(&self) -> BodyLength {
        BodyLength::Chunked
    }
}

impl<'b, B> PrivateBody<'b> for ChunkedBody<'b, B>
where
    B: AsyncIterator<Item = io::Result<&'b [u8]>>,
{
    type WriteMessage<'s, 'h> = SendChunkedBody<'s, 'h, 'b, B>
    where Self: 'b;

    fn write_message<'s, 'h>(
        self,
        stream: &'s mut TcpStream,
        head: &'h [u8],
    ) -> Self::WriteMessage<'s, 'h>
    where
        Self: 'b,
    {
        SendChunkedBody {
            stream,
            body: self.body,
            head,
            body_bytes: None,
            written_chunk_size: false,
        }
    }
}

/// Body that sends the entire file `F`.
#[derive(Debug)]
pub struct FileBody<'f, F> {
    file: &'f F,
    /// Start offset into the `file`.
    offset: usize,
    /// Length of the file, or the maximum number of bytes to send (minus
    /// `offset`).
    /// Always: `end >= offset`.
    end: NonZeroUsize,
}

impl<'f, F> FileBody<'f, F>
where
    F: FileSend,
{
    /// Use a file as HTTP body.
    ///
    /// This uses the bytes `offset..end` from `file` as HTTP body and sends
    /// them using `sendfile(2)` (using [`TcpStream::send_file`]).
    pub const fn new(file: &'f F, offset: usize, end: NonZeroUsize) -> FileBody<'f, F> {
        debug_assert!(end.get() >= offset);
        FileBody { file, offset, end }
    }
}

impl<'f, F> Body<'f> for FileBody<'f, F>
where
    F: FileSend,
{
    fn length(&self) -> BodyLength {
        // NOTE: per the comment on `end`: `end >= offset`, so this can't
        // underflow.
        BodyLength::Known(self.end.get() - self.offset)
    }
}

impl<'f, F> PrivateBody<'f> for FileBody<'f, F>
where
    F: FileSend,
{
    type WriteMessage<'s, 'h> = SendFileBody<'s, 'h, 'f, F>
    where Self: 'f;

    fn write_message<'s, 'h>(
        self,
        stream: &'s mut TcpStream,
        head: &'h [u8],
    ) -> Self::WriteMessage<'s, 'h>
    where
        Self: 'f,
    {
        SendFileBody {
            stream,
            head,
            file: self.file,
            offset: self.offset,
            end: self.end,
        }
    }
}
