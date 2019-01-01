//! Module containing the implementation of the `Process` trait for
//! `Initiator`s.

use std::fmt;
use std::pin::Pin;
use std::time::{Duration, Instant};

use log::{error, trace};

use crate::initiator::Initiator;
use crate::scheduler::process::{Priority, Process, ProcessId, ProcessResult};
use crate::system::ActorSystemRef;

/// A process that represents an [`Initiator`].
///
/// [`Initiator`]: ../../initiator/trait.Initiator.html
pub struct InitiatorProcess<I> {
    id: ProcessId,
    initiator: I,
    runtime: Duration,
}

impl<I> InitiatorProcess<I> {
    /// Create a new `InitiatorProcess`.
    ///
    /// The `initiator` must be initialised, i.e. `init` must have been called
    /// before it's passed to this function.
    pub const fn new(id: ProcessId, initiator: I) -> InitiatorProcess<I> {
        InitiatorProcess {
            id,
            initiator,
            runtime: Duration::from_millis(0),
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
        // Initiators always have a low priority, this way requests in progress
        // are first handled before new requests are accepted and possibly
        // overload the system.
        Priority::LOW
    }

    fn runtime(&self) -> Duration {
        self.runtime
    }

    fn run(self: Pin<&mut Self>, system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running initiator process: pid={}", self.id);

        let start = Instant::now();
        // This is safe because we're not moving the initiator.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        let result = if let Err(err) = this.initiator.poll(system_ref) {
            error!("error polling initiator: {}", err);
            ProcessResult::Complete
        } else {
            ProcessResult::Pending
        };
        let elapsed = start.elapsed();

        trace!("finished running initiator process: pid={}, elapsed_time={:?}", this.id, elapsed);
        this.runtime += elapsed;
        result
    }
}

impl<I> fmt::Debug for InitiatorProcess<I> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("InitiatorProcess")
            .field("id", &self.id)
            .field("runtime", &self.runtime)
            .finish()
    }
}
