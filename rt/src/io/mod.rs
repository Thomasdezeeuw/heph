//! Type definitions for I/O functionality.
//!
//! # Working with Buffers
//!
//! For working with buffers we define two plus two traits. For "regular", i.e.
//! single buffer I/O, we have the following traits:
//!  * [`Buf`] is used in writing/sending.
//!  * [`BufMut`] is used in reading/receiving.
//!
//! The basic design of both traits is the same and is fairly simple. Usage
//! starts with a call to [`parts`]/[`parts_mut`], which returns a pointer to
//! the bytes in the bufer to read from or writing into. For `BufMut` the caller
//! an write into the buffer and update the length using [`update_length`],
//! though normally this is done by an I/O operation.
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

// For ease of use within the crate.
pub(crate) use std::io::{Error, Result};

mod buf;
pub(crate) use buf::BufWrapper;
pub use buf::{Buf, BufMut, BufMutSlice, BufSlice, Limited};

mod futures;
pub(crate) use futures::{
    Read, ReadN, ReadNVectored, ReadVectored, Write, WriteAll, WriteAllVectored, WriteVectored,
};
