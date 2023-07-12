//! Type definitions for I/O functionality.
//!
//! # Working with I/O
//!
//! For working with I/O we define two traits:
//!  * [`Read`]: for reading bytes.
//!  * [`Write`]: for writing bytes.
//!
//! They are similar to the [`std::io::Read`] and [`std::io::Write`] traits, but
//! are asynchronous instead of blocking.
//!
//! Unlike the blocking `Read` and `Write` traits they also take ownership of
//! the buffers, instead of accepting `&mut [u8]`, as that is required to ensure
//! the buffer is not deallocated while it's still used in I/O. For more
//! information about using buffers see the next section.
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
//! # Working with Standard I/O Streams
//!
//! The [`stdin`], [`stdout`] and [`stderr`] function provide handles to
//! standard I/O streams of all Unix processes. All I/O performed using these
//! handles will use io_uring.
//!
//! Note that these handles are **not** buffered, unlike the ones found in the
//! standard library (e.g. [`std::io::stdout`]). Furthermore these handle do not
//! flush the buffer used by the standard library, so it's not advised to use
//! both the handle from standard library and Heph simultaneously.

use crate::access::Access;

mod buf;
pub(crate) use buf::BufWrapper;
pub use buf::{Buf, BufMut, BufMutSlice, BufSlice, Limited};

pub(crate) mod futures;

mod traits;
pub use traits::{Read, Write};

macro_rules! stdio {
    (
        $fn: ident () -> $name: ident
    ) => {
        #[doc = concat!("Create a new `", stringify!($name), "`.\n\n")]
        pub fn $fn<RT: Access>(rt: &RT) -> $name {
            let fd = a10::io::$fn(rt.submission_queue());
            $name { fd }
        }

        #[doc = concat!(
            "A handle for ", stringify!($fn), " of the process.\n\n",
            "# Notes\n\n",
            "This directly writes to the raw file descriptor, which means it's not buffered and will not flush anything buffered by the standard library.\n\n",
            "When this type is dropped it will not close ", stringify!($fn), ".",
        )]
        #[derive(Debug)]
        pub struct $name {
            fd: a10::io::$name,
        }
    };
}

stdio!(stdin() -> Stdin);
stdio!(stdout() -> Stdout);
stdio!(stderr() -> Stderr);

/// Macro to implement the [`Read`] trait using the `fd: a10::AsyncFd` field.
macro_rules! impl_read {
    ( $( $name: ty ),+) => {
        $(
        impl $crate::io::Read for $name {
            async fn read<B: $crate::io::BufMut>(&mut self, buf: B) -> ::std::io::Result<B> {
                $crate::io::futures::Read(self.fd.read($crate::io::BufWrapper(buf))).await
            }

            async fn read_n<B: $crate::io::BufMut>(&mut self, buf: B, n: usize) -> ::std::io::Result<B> {
                debug_assert!(
                    buf.spare_capacity() >= n,
                    concat!("called `", stringify!($name), "::read_n` with a buffer smaller than `n`"),
                );
                $crate::io::futures::ReadN(self.fd.read_n($crate::io::BufWrapper(buf), n)).await
            }

            fn is_read_vectored(&self) -> bool {
                true
            }

            async fn read_vectored<B: $crate::io::BufMutSlice<N>, const N: usize>(
                &mut self,
                bufs: B,
            ) -> ::std::io::Result<B> {
                $crate::io::futures::ReadVectored(self.fd.read_vectored($crate::io::BufWrapper(bufs))).await
            }

            async fn read_n_vectored<B: $crate::io::BufMutSlice<N>, const N: usize>(
                &mut self,
                bufs: B,
                n: usize,
            ) -> ::std::io::Result<B> {
                debug_assert!(
                    bufs.total_spare_capacity() >= n,
                    concat!("called `", stringify!($name), "::read_n_vectored` with buffers smaller than `n`"),
                );
                $crate::io::futures::ReadNVectored(self.fd.read_n_vectored($crate::io::BufWrapper(bufs), n)).await
            }
        }
        )+
    };
}

/// Macro to implement the [`Write`] trait using the `fd: a10::AsyncFd` field.
macro_rules! impl_write {
    ( $( $name: ty ),+) => {
        $(
        impl $crate::io::Write for $name {
            async fn write<B: $crate::io::Buf>(&mut self, buf: B) -> ::std::io::Result<(B, usize)> {
                $crate::io::futures::Write(a10::Extract::extract(self.fd.write($crate::io::BufWrapper(buf)))).await
            }

            async fn write_all<B: $crate::io::Buf>(&mut self, buf: B) -> ::std::io::Result<B> {
                $crate::io::futures::WriteAll(a10::Extract::extract(self.fd.write_all($crate::io::BufWrapper(buf)))).await
            }

            fn is_write_vectored(&self) -> bool {
                true
            }

            async fn write_vectored<B: $crate::io::BufSlice<N>, const N: usize>(
                &mut self,
                bufs: B,
            ) -> ::std::io::Result<(B, usize)> {
                $crate::io::futures::WriteVectored(a10::Extract::extract(self.fd.write_vectored($crate::io::BufWrapper(bufs)))).await
            }

            async fn write_vectored_all<B: $crate::io::BufSlice<N>, const N: usize>(&mut self, bufs: B) -> ::std::io::Result<B> {
                $crate::io::futures::WriteAllVectored(a10::Extract::extract(self.fd.write_all_vectored($crate::io::BufWrapper(bufs)))).await
            }
        }
        )+
    };
}

pub(crate) use {impl_read, impl_write};

impl_read!(Stdin, &Stdin);
impl_write!(Stdout, &Stdout);
impl_write!(Stderr, &Stderr);
