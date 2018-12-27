//! Network related types.

use std::io;

mod tcp;

pub use self::tcp::{TcpStream, TcpListener};

/// Whether or not the error is a would block error.
pub(crate) fn would_block(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

/// Whether or not the error is an interrupted error.
pub(crate) fn interrupted(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Interrupted
}
