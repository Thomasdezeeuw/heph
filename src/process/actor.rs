//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::fmt;
use std::pin::Pin;
use std::task::{LocalWaker, Poll};

use log::{log, trace};

use crate::actor::Actor;
use crate::process::{Process, ProcessResult};
use crate::system::ActorSystemRef;

/// A process that represent an [`Actor`].
///
/// [`Actor`]: ../../actor/trait.Actor.html
pub struct ActorProcess<A> {
    /// The actor.
    actor: A,
    /// Waker used in the futures context.
    waker: LocalWaker,
}

impl<A> ActorProcess<A> {
    /// Create a new `ActorProcess`.
    pub(crate) fn new(actor: A, waker: LocalWaker) -> ActorProcess<A> {
        ActorProcess {
            actor,
            waker,
        }
    }
}

impl<A> Process for ActorProcess<A>
    where A: Actor,
{
    fn run(&mut self, _system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running actor process");

        // FIXME: Currently this is safe because `ProcessData` in the scheduler
        // module boxes each process, but this needs improvement. Maybe go the
        // future route: `self: PinMut<Self>`.
        let actor = unsafe { Pin::new_unchecked(&mut self.actor) };

        match actor.try_poll(&self.waker) {
            Poll::Ready(Ok(())) => ProcessResult::Complete,
            Poll::Ready(Err(_err)) => {
                // TODO: send error to supervisor.
                ProcessResult::Complete
            },
            Poll::Pending => ProcessResult::Pending,
        }
    }
}

impl<A> fmt::Debug for ActorProcess<A> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorProcess")
            .finish()
    }
}
