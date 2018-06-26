//! Module containing the implementation of the `Process` trait for `Initiator`s.

use std::fmt;

use initiator::Initiator;
use process::{Process, ProcessCompletion};
use system::ActorSystemRef;

/// A process that represent an initiator.
pub struct InitiatorProcess<I> {
    /// The initiator.
    initiator: I,
}

impl<I> InitiatorProcess<I>
    where I: Initiator,
{
    /// Create a new process.
    pub const fn new(initiator: I) -> InitiatorProcess<I> {
        InitiatorProcess {
            initiator,
        }
    }
}

impl<I> Process for InitiatorProcess<I>
    where I: Initiator,
{
    fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessCompletion {
        if let Err(err) = self.initiator.poll(system_ref) {
            error!("error polling initiator: {}", err);
            ProcessCompletion::Complete
        } else {
            ProcessCompletion::Pending
        }
    }
}

impl<I> fmt::Debug for InitiatorProcess<I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: add fields.
        f.debug_struct("InitiatorProcess")
            .finish()
    }
}
