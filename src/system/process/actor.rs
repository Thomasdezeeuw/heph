//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::pin::Pin;
use std::task::{self, Poll};

use crate::inbox::Inbox;
use crate::supervisor::SupervisorStrategy;
use crate::system::process::{Process, ProcessId, ProcessResult};
use crate::{actor, Actor, ActorSystemRef, NewActor, Supervisor};

/// A process that represent an [`Actor`].
pub struct ActorProcess<S, NA: NewActor> {
    supervisor: S,
    new_actor: NA,
    actor: NA::Actor,
    /// The inbox of the actor, used in create a new `actor::Context` if the
    /// actor is restarted.
    inbox: Inbox<NA::Message>,
}

impl<S, NA: NewActor> ActorProcess<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor + 'static,
{
    /// Create a new `ActorProcess`.
    pub(crate) const fn new(
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Inbox<NA::Message>,
    ) -> ActorProcess<S, NA> {
        ActorProcess {
            supervisor,
            new_actor,
            actor,
            inbox,
        }
    }

    /// Returns `Ok(true)` if the actor was restarted, `Ok(false)` if the actor
    /// wasn't restarted and an error if the actor failed to restart.
    fn handle_actor_error(
        &mut self,
        system_ref: &mut ActorSystemRef,
        pid: ProcessId,
        err: <NA::Actor as Actor>::Error,
    ) -> Result<bool, NA::Error> {
        match self.supervisor.decide(err) {
            SupervisorStrategy::Restart(arg) => {
                self.create_new_actor(system_ref, pid, arg).map(|()| true)
            }
            SupervisorStrategy::Stop => Ok(false),
        }
    }

    /// Same `handle_actor_error` but handles `NewActor::Error`s instead.
    fn handle_restart_error(
        &mut self,
        system_ref: &mut ActorSystemRef,
        pid: ProcessId,
        err: NA::Error,
    ) -> Result<bool, NA::Error> {
        match self.supervisor.decide_on_restart_error(err) {
            SupervisorStrategy::Restart(arg) => {
                self.create_new_actor(system_ref, pid, arg).map(|()| true)
            }
            SupervisorStrategy::Stop => Ok(false),
        }
    }

    /// Create a new actor and, if successful, replace the old actor with it.
    fn create_new_actor(
        &mut self,
        system_ref: &mut ActorSystemRef,
        pid: ProcessId,
        arg: NA::Argument,
    ) -> Result<(), NA::Error> {
        // Create a new actor.
        let ctx = actor::Context::new(pid, system_ref.clone(), self.inbox.clone());
        self.new_actor.new(ctx, arg).map(|actor| self.actor = actor)
    }
}

impl<S, NA> Process for ActorProcess<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor + 'static,
{
    fn run(self: Pin<&mut Self>, system_ref: &mut ActorSystemRef, pid: ProcessId) -> ProcessResult {
        // This is safe because we're not moving any values.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        // The actor need to be called with `Pin`. So we're undoing the previous
        // operation, still ensuring that the actor is not moved.
        let mut pinned_actor = unsafe { Pin::new_unchecked(&mut this.actor) };

        let waker = system_ref.new_waker(pid);
        let mut ctx = task::Context::from_waker(&waker);
        match Actor::try_poll(pinned_actor.as_mut(), &mut ctx) {
            Poll::Ready(Ok(())) => ProcessResult::Complete,
            Poll::Ready(Err(err)) => match this.handle_actor_error(system_ref, pid, err) {
                Ok(true) => {
                    // Run the actor, just in case progress can be made already,
                    // this required because we use edge triggers for I/O.
                    unsafe { Pin::new_unchecked(this) }.run(system_ref, pid)
                }
                // Actor wasn't restarted.
                Ok(false) => ProcessResult::Complete,
                Err(err) => match this.handle_restart_error(system_ref, pid, err) {
                    Ok(true) => {
                        // Run the actor.
                        unsafe { Pin::new_unchecked(this) }.run(system_ref, pid)
                    }
                    // Actor wasn't restarted.
                    Ok(false) => ProcessResult::Complete,
                    Err(err) => {
                        this.supervisor.second_restart_error(err);
                        ProcessResult::Complete
                    }
                },
            },
            Poll::Pending => ProcessResult::Pending,
        }
    }
}
