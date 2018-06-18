//! Module containing the implementation of the `Process` trait for `Initiator`s.

use initiator::Initiator;
use system::ActorSystemRef;
use system::process::{Process, ProcessId, ProcessCompletion};
use system::scheduler::Priority;

/// A process that represent an initiator.
#[derive(Debug)]
pub struct InitiatorProcess<I> {
    /// Unique id in the `ActorSystem`.
    id: ProcessId,
    /// The priority of the process.
    priority: Priority,
    /// The initiator.
    initiator: I,
}

impl<I> InitiatorProcess<I>
    where I: Initiator,
{
    /// Create a new process.
    pub fn new(id: ProcessId, initiator: I) -> InitiatorProcess<I> {
        InitiatorProcess {
            id,
            // Give initiators a low priority to first handle old requests,
            // before accepting new ones.
            priority: Priority::LOW,
            initiator,
        }
    }
}

impl<I> Process for InitiatorProcess<I>
    where I: Initiator,
{
    fn id(&self) -> ProcessId {
        self.id
    }

    fn priority(&self) -> Priority {
        self.priority
    }

    fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessCompletion {
        if let Err(err) = self.initiator.poll(system_ref) {
            error!("error polling initiator: {}", err);
            ProcessCompletion::Complete
        } else {
            ProcessCompletion::Pending
        }
    }
}
