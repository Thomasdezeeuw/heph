//! Module with HTTP body related types.
//!
//! See the [`Body`] trait.

use std::io::{self, IoSlice};
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::stream::Stream;

use heph::net::tcp::stream::{FileSend, SendAll, TcpStream};

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
pub trait Body: PrivateBody {
    /// Length of the body, or the body will be chunked.
    fn length(&self) -> BodyLength;
}

/// Length of a body.
pub enum BodyLength {
    /// Body length is known.
    Known(usize),
    /// Body length is unknown and the body will be transfered using chunked
    /// encoding.
    Chunked,
}

mod private {
    use std::future::Future;
    use std::io::{self, IoSlice};
    use std::num::NonZeroUsize;
    use std::pin::Pin;
    use std::stream::Stream;
    use std::task::{self, Poll};

    use heph::net::tcp::stream::FileSend;
    use heph::net::TcpStream;

    /// Private extention of [`PrivateBody`].
    pub trait PrivateBody {
        type WriteBody<'s, 'b>: Future<Output = io::Result<()>>;

        /// Write the response to `stream`.
        ///
        /// The `http_head` buffer contains the HTTP header (i.e. status line
        /// and all headers), this must still be written to the `stream` also.
        fn write_response<'s, 'b>(
            &'b mut self,
            stream: &'s mut TcpStream,
            http_head: &'b [u8],
        ) -> Self::WriteBody<'s, 'b>;
    }

    /// See [`OneshotBody`].
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
                            IoSlice::advance(&mut bufs[..1], n);
                        } else {
                            // Written entire head.
                            bufs[0] = IoSlice::new(&[]);
                            IoSlice::advance(&mut bufs[1..], n - head_len);
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

    /// See [`StreamingBody`].
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
        B: Stream<Item = io::Result<&'b [u8]>>,
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

            while *left != 0 {
                // We have bytes we need to send.
                if let Some(bytes) = body_bytes.as_mut() {
                    match stream.try_send(*bytes) {
                        Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                        Ok(n) if n >= bytes.len() => {
                            *body_bytes = None;
                        }
                        Ok(n) => {
                            *bytes = &bytes[n..];
                            continue;
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

    /// See [`FileBody`].
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

pub(crate) use private::PrivateBody;
use private::{SendFileBody, SendOneshotBody, SendStreamingBody};

/// An empty body.
#[derive(Debug)]
pub struct EmptyBody;

impl Body for EmptyBody {
    fn length(&self) -> BodyLength {
        BodyLength::Known(0)
    }
}

impl PrivateBody for EmptyBody {
    type WriteBody<'s, 'b> = SendAll<'s, 'b>;

    fn write_response<'s, 'b>(
        &'b mut self,
        stream: &'s mut TcpStream,
        http_head: &'b [u8],
    ) -> Self::WriteBody<'s, 'b> {
        // Just need to write the HTTP head as we don't have a body.
        stream.send_all(http_head)
    }
}

/// Body length and content is known in advance. Send in a single payload (i.e.
/// not chunked).
#[derive(Debug)]
pub struct OneshotBody<'b> {
    bytes: &'b [u8],
}

impl<'b> OneshotBody<'b> {
    /// Create a new one-shot body.
    pub const fn new(body: &'b [u8]) -> OneshotBody<'b> {
        OneshotBody { bytes: body }
    }
}

impl<'b> Body for OneshotBody<'b> {
    fn length(&self) -> BodyLength {
        BodyLength::Known(self.bytes.len())
    }
}

impl<'a> PrivateBody for OneshotBody<'a> {
    type WriteBody<'s, 'b> = SendOneshotBody<'s, 'b>;

    fn write_response<'s, 'b>(
        &'b mut self,
        stream: &'s mut TcpStream,
        http_head: &'b [u8],
    ) -> Self::WriteBody<'s, 'b> {
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

/// Streaming body with a known length. Send in a single payload (i.e. not
/// chunked).
#[derive(Debug)]
pub struct StreamingBody<'b, B> {
    length: usize,
    body: Option<B>,
    _body_lifetime: PhantomData<&'b [u8]>,
}

impl<'b, B> Body for StreamingBody<'b, B>
where
    B: Stream<Item = io::Result<&'b [u8]>>,
{
    fn length(&self) -> BodyLength {
        BodyLength::Known(self.length)
    }
}

impl<'b, B> PrivateBody for StreamingBody<'b, B>
where
    B: Stream<Item = io::Result<&'b [u8]>>,
{
    type WriteBody<'s, 'h> = SendStreamingBody<'s, 'h, 'b, B>;

    fn write_response<'s, 'h>(
        &'h mut self,
        stream: &'s mut TcpStream,
        head: &'h [u8],
    ) -> Self::WriteBody<'s, 'h> {
        SendStreamingBody {
            stream,
            body: self.body.take().unwrap(),
            head,
            left: self.length,
            body_bytes: None,
        }
    }
}

/// Streaming body with an unknown length. Send in multiple chunks.
#[derive(Debug)]
pub struct ChunkedBody<'b, B> {
    stream: B,
    _body_lifetime: PhantomData<&'b [u8]>,
}

// TODO: implement `Body` for `ChunkedBody`.

/// Body that sends the entire file `F`.
pub struct FileBody<'f, F> {
    file: Option<&'f F>,
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
    /// Use a file as HTTP body
    ///
    /// This uses the bytes `offset..end` from `file` as HTTP body and sends
    /// them using `sendfile(2)` (using [`TcpStream::send_file`]).
    // TODO: make this a `const` fn once trait bounds (`FileSend`) on `const`
    // functions are stable.
    pub fn new(file: &'f F, offset: usize, end: NonZeroUsize) -> FileBody<'f, F> {
        debug_assert!(end.get() >= offset);
        FileBody {
            file: Some(file),
            offset,
            end,
        }
    }
}

impl<'f, F> Body for FileBody<'f, F>
where
    F: FileSend,
{
    fn length(&self) -> BodyLength {
        // NOTE: per the comment on `end`: `end >= offset`, so this can't
        // underflow.
        BodyLength::Known(self.end.get() - self.offset)
    }
}

impl<'f, F> PrivateBody for FileBody<'f, F>
where
    F: FileSend,
{
    type WriteBody<'s, 'h> = SendFileBody<'s, 'h, 'f, F>;

    fn write_response<'s, 'h>(
        &'h mut self,
        stream: &'s mut TcpStream,
        head: &'h [u8],
    ) -> Self::WriteBody<'s, 'h> {
        SendFileBody {
            stream,
            head,
            file: self.file.take().unwrap(),
            offset: self.offset,
            end: self.end,
        }
    }
}
