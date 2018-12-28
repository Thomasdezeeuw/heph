//! Module containing the implementation of the `Process` trait for
//! `Initiator`s.

use std::fmt;
use std::pin::Pin;

use log::{error, trace};

use crate::initiator::Initiator;
use crate::process::{Process, ProcessResult};
use crate::system::ActorSystemRef;

/// A process that represents an [`Initiator`].
///
/// [`Initiator`]: ../../initiator/trait.Initiator.html
pub struct InitiatorProcess<I> {
    initiator: I,
}

impl<I> InitiatorProcess<I> {
    /// Create a new `InitiatorProcess`.
    ///
    /// The `initiator` must be initialised, i.e. `init` must have been called
    /// before it's passed to this function.
    pub const fn new(initiator: I) -> InitiatorProcess<I> {
        InitiatorProcess {
            initiator,
        }
    }
}

impl<I> Process for InitiatorProcess<I>
    where I: Initiator,
{
    fn run(self: Pin<&mut Self>, system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running initiator process");

        // This is safe because we're not moving the initiator.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        if let Err(err) = this.initiator.poll(system_ref) {
            error!("error polling initiator: {}", err);
            ProcessResult::Complete
        } else {
            ProcessResult::Pending
        }
    }
}

impl<I> fmt::Debug for InitiatorProcess<I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("InitiatorProcess")
            .finish()
    }
}
