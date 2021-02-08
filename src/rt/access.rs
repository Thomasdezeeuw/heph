//! Module to that defines the [`rt::Access`] trait.
//!
//! [`rt::Access`]: crate::rt::Access

use std::io;
use std::time::Instant;

use mio::{event, Interest, Token};

use crate::rt::{ProcessId, Waker};

/// Trait to indicate an API needs access to the Heph runtime.
///
/// This is used by various API to get access to the runtime, but its only
/// usable inside the Heph crate.
///
/// # Notes
///
/// This trait can't be implemented by types outside of the Heph crate.
pub trait Access: PrivateAccess {}

impl<T> Access for T where T: PrivateAccess {}

/// Actual trait behind [`rt::Access`].
///
/// [`rt::Access`]: crate::rt::Access
pub trait PrivateAccess {
    /// Create a new [`Waker`].
    fn new_waker(&mut self, pid: ProcessId) -> Waker;

    /// Registers the `source` using `token` and `interest`.
    fn register<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized;

    /// Reregisters the `source` using `token` and `interest`.
    fn reregister<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized;

    /// Add a deadline for `pid` at `deadline`.
    fn add_deadline(&mut self, pid: ProcessId, deadline: Instant);

    /// Returns the CPU the thread is bound to, if any.
    fn cpu(&self) -> Option<usize>;
}
