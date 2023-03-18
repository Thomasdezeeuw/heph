//! Type definitions for I/O functionality.
//!
//! The main types of this module are the [`Buf`] and [`BufMut`] traits, which
//! define the requirements on buffers in use in I/O. Additionally the
//! [`BufSlice`] and [`BufMutSlice`] traits define the behaviour of buffers in
//! vectored I/O.

// For ease of use within the crate.
pub(crate) use std::io::{Error, Result};

mod buf;
pub use buf::{Buf, BufMut, BufMutSlice, BufSlice};
