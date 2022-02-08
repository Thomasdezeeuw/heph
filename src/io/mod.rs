//! Module with I/O types and traits.
//!
//! The main trait is [`Stream`] which defines a reliable, ordered, and
//! error-checked delivery of a stream of bytes. The trait itself is split into
//! [`RecvStream`] and [`SendStream`], which represent the receiving and sending
//! side respectively.
//!
//! Next there is the [`PeekStream`] which allows receiving bytes without
//! removing them from its input queue.
//!
//! Finally there is the [`SendFile`] trait which allows a file to be send to
//! the stream as optimisation.

use std::future::Future;
use std::io::{self, IoSlice};
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{self, Poll};

pub mod bytes;

use bytes::{Bytes, BytesVectored};

/// Reliable, ordered, and error-checked delivery of a stream of bytes.
///
/// This can be a TCP stream, but could also be Unix Domain Socket (UDS) or any
/// kind stream of bytes.
pub trait Stream: RecvStream + SendStream {}

impl<T> Stream for T where T: RecvStream + SendStream {}

/// Receiving (reading) side of a [`Stream`].
#[doc(alias = "Read")]
pub trait RecvStream {
    /// [`Future`] behind [`recv`].
    ///
    /// [`recv`]: RecvStream::recv
    type Recv<'a, B>: Future<Output = io::Result<usize>>
    where
        B: Bytes;

    /// Receive messages from the stream, writing them into `buf`.
    fn recv<'a, B>(&'a mut self, buf: B) -> Self::Recv<'a, B>
    where
        B: Bytes;

    /// [`Future`] behind [`recv_vectored`].
    ///
    /// [`recv_vectored`]: RecvStream::recv_vectored
    type RecvVectored<'a, B>: Future<Output = io::Result<usize>>
    where
        B: BytesVectored;

    /// [Receiving] using vectored I/O.
    ///
    /// [Receiving]: RecvStream::recv
    fn recv_vectored<'a, B>(&'a mut self, bufs: B) -> Self::RecvVectored<'a, B>
    where
        B: BytesVectored;
}

/// Peeking of a [`Stream`].
///
/// This is similar to [`RecvStream`], but doesn't remove the received (peeked)
/// byes from its buffers, allowing them to be received again.
#[doc(alias = "Peek")]
pub trait PeekStream {
    /// [`Future`] behind [`peek`].
    ///
    /// [`peek`]: PeekStream::peek
    type Peek<'a, B>: Future<Output = io::Result<usize>>
    where
        B: Bytes;

    /// Receive bytes from the stream, writing them into `buf`, without removing
    /// that data from the queue. On success, returns the number of bytes
    /// peeked.
    fn peek<'a, B>(&'a mut self, buf: B) -> Self::Peek<'a, B>
    where
        B: Bytes;

    /// [`Future`] behind [`peek_vectored`].
    ///
    /// [`peek_vectored`]: PeekStream::peek_vectored
    type PeekVectored<'a, B>: Future<Output = io::Result<usize>>
    where
        B: BytesVectored;

    /// [Peeking] using vectored I/O.
    ///
    /// [Peeking]: PeekStream::peek
    fn peek_vectored<'a, B>(&'a mut self, bufs: B) -> Self::PeekVectored<'a, B>
    where
        B: BytesVectored;
}

/// Sending (writing) side of a [`Stream`].
#[doc(alias = "Write")]
pub trait SendStream {
    /// [`Future`] behind [`send`].
    ///
    /// [`send`]: SendStream::send
    type Send<'a, 'b>: Future<Output = io::Result<usize>>;

    /// Send the bytes in `buf` into the stream.
    ///
    /// Return the number of bytes written. This may we fewer then the length of
    /// `buf`. To ensure that all bytes are written use [`SendStream::send_all`].
    fn send<'a, 'b>(&'a mut self, buf: &'b [u8]) -> Self::Send<'a, 'b>;

    /// Send the all bytes in `buf` into the stream.
    ///
    /// If this fails to send all bytes (this happens if a send returns `Ok(0)`)
    /// this will return [`io::ErrorKind::WriteZero`].
    fn send_all<'a, 'b>(&'a mut self, buf: &'b [u8]) -> SendAll<'a, 'b, Self>
    where
        Self: Sized,
    {
        SendAll {
            stream: self,
            buf,
            pending: self.send(buf),
        }
    }

    /// [`Future`] behind [`send_vectored`].
    ///
    /// [`send_vectored`]: SendStream::send_vectored
    type SendVectored<'a, 'b>: Future<Output = io::Result<usize>>;

    /// [Sending] using vectored I/O.
    ///
    /// [Sending]: SendStream::send
    fn send_vectored<'a, 'b>(
        &'a mut self,
        bufs: &'b mut [IoSlice<'b>],
    ) -> Self::SendVectored<'a, 'b>;
}

/// The [`Future`] behind [`SendStream::send_all`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendAll<'a, 'b, S>
where
    S: SendStream,
{
    stream: &'a mut S,
    buf: &'b [u8],
    pending: S::Send<'a, 'b>,
}

impl<'a, 'b, S> Future for SendAll<'a, 'b, S>
where
    S: SendStream,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: we can't move the `Future` in `pending` or `stream`.
        let this = unsafe { Pin::into_inner_unchecked(self) };
        loop {
            /*
            // SAFETY: can't move `fut`!
            let fut = match this.pending.as_mut() {
                Some(fut) => fut,
                None => {
                    let fut: S::Send<'a, 'b> = this.stream.send(this.buf);
                    this.pending.insert(fut)
                }
            };
            */
            /*
            match unsafe { Pin::new_unchecked(fut).poll(ctx) } {
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Poll::Ready(Ok(n)) if this.buf.len() <= n => return Poll::Ready(Ok(())),
                Poll::Ready(Ok(n)) => {
                    this.buf = &this.buf[n..];
                    // Try to send some more bytes.
                    this.pending = None;
                    continue;
                }
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Pending => return Poll::Pending,
            }
            */
        }
        todo!()
    }
}

/// An optimisation trait to allow a file to send to a stream.
///
/// This should be seen as an extension of [`SendStream`].
pub trait SendFile {
    /// [`Future`] behind [`sendfile`].
    ///
    /// [`sendfile`]: SendFile::sendfile
    type SendFile<'a, F>: Future<Output = io::Result<usize>>;

    /// Send the `file` out this stream.
    ///
    /// What kind of files are support depends on the OS and is determined by
    /// the [`FileSend`] trait. All OSs at least support regular files.
    ///
    /// The `offset` is the offset into the `file` from which to start copying.
    /// The `length` is the amount of bytes to copy, or if `None` this send the
    /// entire `file`.
    fn sendfile<'a, 'b, F>(
        &mut self,
        file: &'b F,
        offset: usize,
        length: Option<NonZeroUsize>,
    ) -> io::Result<usize>
    where
        F: FileSend;
}

/// Trait that determines which types are safe to use in [`SendFile`].
pub trait FileSend: private::PrivateFileSend {}

mod private {
    use std::fs::File;
    use std::os::unix::io::AsRawFd;

    /// Private version of [`FileSend`].
    ///
    /// [`FileSend`]: super::FileSend
    pub trait PrivateFileSend: AsRawFd {}

    impl super::FileSend for File {}

    impl PrivateFileSend for File {}
}
