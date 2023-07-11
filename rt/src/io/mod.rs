//! Type definitions for I/O functionality.
//!
//! # Working with Buffers
//!
//! For working with buffers we define two plus two traits. For "regular", i.e.
//! single buffer I/O, we have the following two traits:
//!  * [`Buf`] is used in writing/sending.
//!  * [`BufMut`] is used in reading/receiving.
//!
//! The basic design of both traits is the same and is fairly simple. Usage
//! starts with a call to [`parts`]/[`parts_mut`], which returns a pointer to
//! the bytes in the bufer to read from or write into. For `BufMut` the caller
//! writes into the buffer and updates the length using [`update_length`], though
//! normally this is done by an I/O operation.
//!
//! For vectored I/O we have the same two traits as above, but suffixed with
//! `Slice`:
//!  * [`BufSlice`] is used in vectored writing/sending.
//!  * [`BufMutSlice`] is used in vectored reading/receiving.
//!
//! Neither of these traits can be implemented outside of the crate, but it's
//! already implemented for tuples and arrays.
//!
//! [`parts`]: Buf::parts
//! [`parts_mut`]: BufMut::parts_mut
//! [`update_length`]: BufMut::update_length
//!
//! # Working with Standard I/O Stream
//!
//! The [`stdin`], [`stdout`] and [`stderr`] function provide handles to
//! standard I/O streams of all Unix processes. All I/O performed using these
//! handles will use io_uring.
//!
//! Note that these handles are **not** buffered, unlike the ones found in the
//! standard library (e.g. [`std::io::stdout`]). Furthermore these handle do not
//! flush the buffer used by the standard library, so it's not advised to use
//! both the handle from standard library and Heph simultaneously.

use a10::Extract;

use crate::access::Access;

// For ease of use within the crate.
pub(crate) use std::io::{Error, Result};

mod buf;
pub(crate) use buf::BufWrapper;
pub use buf::{Buf, BufMut, BufMutSlice, BufSlice, Limited};

pub(crate) mod futures;

macro_rules! stdio {
    (
        $fn: ident () -> $name: ident, $fd: expr
    ) => {
        #[doc = concat!("Create a new `", stringify!($name), "`.\n\n")]
        pub fn $fn<RT: Access>(rt: &RT) -> $name {
            $name(std::mem::ManuallyDrop::new(unsafe { a10::AsyncFd::from_raw_fd(
                $fd,
                rt.submission_queue(),
            )}))
        }

        #[doc = concat!(
            "A handle for ", stringify!($fn), " of the process.\n\n",
            "# Notes\n\n",
            "This directly writes to the raw file descriptor, which means it's not buffered and will not flush anything buffered by the standard library.\n\n",
            "When this type is dropped it will not close ", stringify!($fn), ".",
        )]
        #[derive(Debug)]
        pub struct $name(std::mem::ManuallyDrop<a10::AsyncFd>);
    };
}

stdio!(stdin() -> Stdin, libc::STDIN_FILENO);
stdio!(stdout() -> Stdout, libc::STDOUT_FILENO);
stdio!(stderr() -> Stderr, libc::STDERR_FILENO);

impl Stdin {
    /// Read bytes from standard in, writing them into `buf`.
    pub async fn read<B: BufMut>(&self, buf: B) -> Result<B> {
        futures::Read(self.0.read(BufWrapper(buf))).await
    }

    /// Read at least `n` bytes from standard in, writing them into `buf`.
    ///
    /// This returns [`io::ErrorKind::UnexpectedEof`] if less than `n` bytes
    /// could be read.
    ///
    /// [`io::ErrorKind::UnexpectedEof`]: std::io::ErrorKind::UnexpectedEof
    pub async fn read_n<B: BufMut>(&self, buf: B, n: usize) -> Result<B> {
        debug_assert!(
            buf.spare_capacity() >= n,
            "called `Receiver::read_n` with a buffer smaller than `n`",
        );
        futures::ReadN(self.0.read_n(BufWrapper(buf), n)).await
    }

    /// Read bytes from standard in, writing them into `bufs`.
    pub async fn read_vectored<B: BufMutSlice<N>, const N: usize>(&self, bufs: B) -> Result<B> {
        futures::ReadVectored(self.0.read_vectored(BufWrapper(bufs))).await
    }

    /// Read at least `n` bytes from standard in, writing them into `bufs`.
    pub async fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &self,
        bufs: B,
        n: usize,
    ) -> Result<B> {
        debug_assert!(
            bufs.total_spare_capacity() >= n,
            "called `Receiver::read_n_vectored` with buffers smaller than `n`"
        );
        futures::ReadNVectored(self.0.read_n_vectored(BufWrapper(bufs), n)).await
    }
}

impl Stdout {
    /// Write the bytes in `buf` to standard out.
    ///
    /// Return the number of bytes written. This may we fewer than the length of
    /// `buf`. To ensure that all bytes are written use [`Stdout::write_all`].
    pub async fn write<B: Buf>(&self, buf: B) -> Result<(B, usize)> {
        futures::Write(self.0.write(BufWrapper(buf)).extract()).await
    }

    /// Write the all bytes in `buf` to standard out.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    ///
    /// [`io::ErrorKind::WriteZero`]: std::io::ErrorKind::WriteZero
    pub async fn write_all<B: Buf>(&self, buf: B) -> Result<B> {
        futures::WriteAll(self.0.write_all(BufWrapper(buf)).extract()).await
    }

    /// Write the bytes in `bufs` to standard out.
    ///
    /// Return the number of bytes written. This may we fewer than the length of
    /// `bufs`. To ensure that all bytes are written use
    /// [`Stdout::write_vectored_all`].
    pub async fn write_vectored<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> Result<(B, usize)> {
        futures::WriteVectored(self.0.write_vectored(BufWrapper(bufs)).extract()).await
    }

    /// Write the all bytes in `bufs` to standard out.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    ///
    /// [`io::ErrorKind::WriteZero`]: std::io::ErrorKind::WriteZero
    pub async fn write_vectored_all<B: BufSlice<N>, const N: usize>(&self, bufs: B) -> Result<B> {
        futures::WriteAllVectored(self.0.write_all_vectored(BufWrapper(bufs)).extract()).await
    }
}

impl Stderr {
    /// Write the bytes in `buf` to standard error.
    ///
    /// Return the number of bytes written. This may we fewer than the length of
    /// `buf`. To ensure that all bytes are written use [`Stderr::write_all`].
    pub async fn write<B: Buf>(&self, buf: B) -> Result<(B, usize)> {
        futures::Write(self.0.write(BufWrapper(buf)).extract()).await
    }

    /// Write the all bytes in `buf` to standard error.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    ///
    /// [`io::ErrorKind::WriteZero`]: std::io::ErrorKind::WriteZero
    pub async fn write_all<B: Buf>(&self, buf: B) -> Result<B> {
        futures::WriteAll(self.0.write_all(BufWrapper(buf)).extract()).await
    }

    /// Write the bytes in `bufs` to standard error.
    ///
    /// Return the number of bytes written. This may we fewer than the length of
    /// `bufs`. To ensure that all bytes are written use
    /// [`Stderr::write_vectored_all`].
    pub async fn write_vectored<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> Result<(B, usize)> {
        futures::WriteVectored(self.0.write_vectored(BufWrapper(bufs)).extract()).await
    }

    /// Write the all bytes in `bufs` to standard error.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    ///
    /// [`io::ErrorKind::WriteZero`]: std::io::ErrorKind::WriteZero
    pub async fn write_vectored_all<B: BufSlice<N>, const N: usize>(&self, bufs: B) -> Result<B> {
        futures::WriteAllVectored(self.0.write_all_vectored(BufWrapper(bufs)).extract()).await
    }
}
