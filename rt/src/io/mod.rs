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
//! writes into the buffer and updates the length using [`set_init`], though
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
//! [`set_init`]: BufMut::set_init
//!
//! [`ReadBufPool`] is a specialised read buffer pool that can only be used in
//! read operations done by the kernel, i.e. no in-memory operations.
//!
//! # Working with Standard I/O Streams
//!
//! The [`stdin`], [`stdout`] and [`stderr`] functions provide handles to
//! standard I/O streams of all Unix processes. All I/O performed using these
//! handles will non-blocking I/O.
//!
//! Note that these handles are **not** buffered, unlike the ones found in the
//! standard library (e.g. [`std::io::stdout`]). Furthermore these handle do not
//! flush the buffer used by the standard library, so it's not advised to use
//! both the handle from standard library and Heph simultaneously.

pub use a10::io::{
    stderr, stdin, stdout, Buf, BufMut, BufMutSlice, BufSlice, Close, Read, ReadBuf, ReadBufPool,
    ReadN, ReadNVectored, ReadVectored, Splice, Stderr, Stdin, Stdout, Write, WriteAll,
    WriteAllVectored, WriteVectored,
};
