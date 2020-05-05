//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::pin::Pin;
use std::task::{self, Poll};

use crate::actor::{self, context, Actor, NewActor};
use crate::inbox::{Inbox, InboxRef};
use crate::rt::process::{Process, ProcessId, ProcessResult};
use crate::supervisor::SupervisorStrategy;
use crate::{RuntimeRef, Supervisor};

/// A process that represent an [`Actor`].
pub struct ActorProcess<S, NA: NewActor> {
    /// The actor's supervisor used to determine what to do when the actor, or
    /// [`NewActor`] implementation, returns an error.
    supervisor: S,
    /// The [`NewActor`] implementation used to restart the actor.
    new_actor: NA,
    /// The inbox of the actor, used in creating a new [`actor::Context`]
    /// if the actor is restarted.
    inbox: Inbox<NA::Message>,
    inbox_ref: InboxRef<NA::Message>,
    /// The running actors.
    actor: NA::Actor,
}

impl<S, NA: NewActor> ActorProcess<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor<Context = context::ThreadLocal>,
{
    /// Create a new `ActorProcess`.
    pub(crate) const fn new(
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Inbox<NA::Message>,
        inbox_ref: InboxRef<NA::Message>,
    ) -> ActorProcess<S, NA> {
        ActorProcess {
            supervisor,
            new_actor,
            actor,
            inbox,
            inbox_ref,
        }
    }

    /// Returns `Ok(true)` if the actor was successfully restarted, `Ok(false)`
    /// if the actor wasn't restarted or an error if the actor failed to
    /// restart.
    fn handle_actor_error(
        &mut self,
        runtime_ref: &mut RuntimeRef,
        pid: ProcessId,
        err: <NA::Actor as Actor>::Error,
    ) -> Result<bool, NA::Error> {
        match self.supervisor.decide(err) {
            SupervisorStrategy::Restart(arg) => {
                self.create_new_actor(runtime_ref, pid, arg).map(|()| true)
            }
            SupervisorStrategy::Stop => Ok(false),
        }
    }

    /// Same as `handle_actor_error` but handles [`NewActor::Error`]s instead.
    fn handle_restart_error(
        &mut self,
        runtime_ref: &mut RuntimeRef,
        pid: ProcessId,
        err: NA::Error,
    ) -> Result<bool, NA::Error> {
        match self.supervisor.decide_on_restart_error(err) {
            SupervisorStrategy::Restart(arg) => {
                self.create_new_actor(runtime_ref, pid, arg).map(|()| true)
            }
            SupervisorStrategy::Stop => Ok(false),
        }
    }

    /// Creates a new actor and, if successful, replaces the old actor with it.
    fn create_new_actor(
        &mut self,
        runtime_ref: &mut RuntimeRef,
        pid: ProcessId,
        arg: NA::Argument,
    ) -> Result<(), NA::Error> {
        // Create a new actor.
        let ctx = actor::Context::new_local(
            pid,
            self.inbox.ctx_inbox(),
            self.inbox_ref.clone(),
            runtime_ref.clone(),
        );
        self.new_actor.new(ctx, arg).map(|actor| {
            // We pin the actor here to ensure its dropped in place when
            // replacing it with out new actor.
            unsafe { Pin::new_unchecked(&mut self.actor) }.set(actor)
        })
    }
}

impl<S, NA> Process for ActorProcess<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor<Context = context::ThreadLocal>,
{
    fn run(self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef, pid: ProcessId) -> ProcessResult {
        // This is safe because we're not moving the actor.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        // The actor need to be called with `Pin`. So we're undoing the previous
        // operation, still ensuring that the actor is not moved.
        let mut actor = unsafe { Pin::new_unchecked(&mut this.actor) };

        let waker = runtime_ref.new_task_waker(pid);
        let mut task_ctx = task::Context::from_waker(&waker);
        match actor.as_mut().try_poll(&mut task_ctx) {
            Poll::Ready(Ok(())) => ProcessResult::Complete,
            Poll::Ready(Err(err)) => match this.handle_actor_error(runtime_ref, pid, err) {
                Ok(true) => {
                    // Run the actor just in case progress can be made already,
                    // this required because we use edge triggers for I/O.
                    unsafe { Pin::new_unchecked(this) }.run(runtime_ref, pid)
                }
                // Actor wasn't restarted.
                Ok(false) => ProcessResult::Complete,
                Err(err) => match this.handle_restart_error(runtime_ref, pid, err) {
                    Ok(true) => {
                        // Run the actor, same reason as above.
                        unsafe { Pin::new_unchecked(this) }.run(runtime_ref, pid)
                    }
                    // Actor wasn't restarted.
                    Ok(false) => ProcessResult::Complete,
                    Err(err) => {
                        // Let the supervisor know.
                        this.supervisor.second_restart_error(err);
                        ProcessResult::Complete
                    }
                },
            },
            Poll::Pending => ProcessResult::Pending,
        }
    }
}
