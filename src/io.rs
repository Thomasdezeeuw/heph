//! Module with I/O traits.

use std::future::Future;
use std::io;
use std::num::NonZeroUsize;

use crate::bytes::Bytes;

/// Receiving (reading) side of a [`Stream`].
#[doc(alias = "Read")]
pub trait RecvStream {
    type Recv<'a, B>: Future<Output = io::Result<usize>>;

    fn recv<'a, B>(&'a mut self, buf: B) -> Self::Recv<'a, B>
    where
        B: Bytes;

    type RecvVectored<'a, B>: Future<Output = io::Result<usize>>;

    fn recv_vectored<'a, B>(&'a mut self, buf: B) -> Self::RecvVectored<'a, B>
    where
        B: Bytes;

    // recv_n
    // recv_n_vectored
}

/// Peeking of a [`Stream`].
///
/// This is similar to [`RecvStream`], but doesn't remove the received (peeked)
/// byes from its buffers, allowing them to be received again.
pub trait PeekStream {
    type Peek<'a, B>: Future<Output = io::Result<usize>>;

    fn peek<'a, B>(&'a mut self, buf: B) -> Self::Peek<'a, B>
    where
        B: Bytes;

    type PeekVectored<'a, B>: Future<Output = io::Result<usize>>;

    fn peek_vectored<'a, B>(&'a mut self, buf: B) -> Self::PeekVectored<'a, B>
    where
        B: Bytes;
}

/// Sending (writing) side of a [`Stream`].
#[doc(alias = "Write")]
pub trait SendStream {
    type Send<'a, 'b>: Future<Output = io::Result<usize>>;

    fn send<'a, 'b>(&'a mut self, buf: &'b [u8]) -> Self::Send<'a, 'b>;

    type SendVectored<'a, 'b>: Future<Output = io::Result<usize>>;

    fn send_vectored<'a, 'b>(&'a mut self, buf: &'b [u8]) -> Self::SendVectored<'a, 'b>;

    // send_all
    // send_vectored_all
}

/// An optimisation trait to allow a file to send to a stream.
///
/// This should be seen as an extension of [`SendStream`].
pub trait SendFile {
    type SendFile<'a, F>: Future<Output = io::Result<usize>>;

    fn sendfile<'a, 'b, F>(
        &mut self,
        file: &'b F,
        offset: usize,
        length: Option<NonZeroUsize>,
    ) -> io::Result<usize>
    //  FIXME: move trait to this module.
    where
        F: crate::net::tcp::stream::FileSend;

    // send_file_all
    // send_entire_file
}

/// Reliable, ordered, and error-checked delivery of a stream of bytes.
///
/// This can be a TCP stream, but could also be Unix Domain Socket (UDS) or any
/// kind stream of bytes.
pub trait Stream: RecvStream + SendStream {}

impl<T> Stream for T where T: RecvStream + SendStream {}
