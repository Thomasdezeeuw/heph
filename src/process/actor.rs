//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::fmt;
use std::mem::PinMut;
use std::task::{Context, Poll, LocalWaker};

use log::{trace, log};
use mio_st::registration::Registration;

use crate::actor::Actor;
use crate::process::{Process, ProcessResult};
use crate::system::ActorSystemRef;

/// A process that represent an actor, it's mailbox and current execution.
pub struct ActorProcess<A>
    where A: Actor,
{
    /// The actor.
    actor: A,
    /// Waker used in the futures context.
    waker: LocalWaker,
    /// Needs to stay alive for the duration of the actor.
    _registration: Registration,
}

impl<A> ActorProcess<A>
    where A: Actor,
{
    /// Create a new `ActorProcess`.
    pub(crate) fn new(actor: A, registration: Registration, waker: LocalWaker) -> ActorProcess<A> {
        ActorProcess {
            actor,
            waker,
            _registration: registration,
        }
    }
}

impl<A> Process for ActorProcess<A>
    where A: Actor,
{
    fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running actor process");

        // FIXME: Currently this is safe because `ProcessData` in the scheduler
        // module boxes each process, but this needs improvement. Maybe go the
        // future route: `self: PinMut<Self>`.
        let actor = unsafe { PinMut::new_unchecked(&mut self.actor) };
        let mut ctx = Context::new(&self.waker, system_ref);

        match actor.try_poll(&mut ctx) {
            Poll::Ready(Ok(())) => ProcessResult::Complete,
            Poll::Ready(Err(_err)) => {
                // TODO: send error to supervisor.
                ProcessResult::Complete
            },
            Poll::Pending => ProcessResult::Pending,
        }
    }
}

impl<A> fmt::Debug for ActorProcess<A>
    where A: Actor,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorProcess")
            .finish()
    }
}
