//! Module containing the implementation of the [`Process`] trait for
//! [`Actor`]s.

use std::pin::Pin;
use std::task::{self, Poll};

use heph_inbox::{Manager, Receiver};

use crate::actor::{self, Actor, NewActor};
use crate::rt::access::PrivateAccess;
use crate::rt::process::{Process, ProcessId, ProcessResult};
use crate::rt::{self, RuntimeRef, ThreadLocal, ThreadSafe};
use crate::supervisor::{Supervisor, SupervisorStrategy};

/// A process that represent an [`Actor`].
pub(crate) struct ActorProcess<S, NA: NewActor> {
    /// The actor's supervisor used to determine what to do when the actor, or
    /// the [`NewActor`] implementation, returns an error.
    supervisor: S,
    /// The [`NewActor`] implementation used to restart the actor.
    new_actor: NA,
    /// The inbox of the actor, used in creating a new [`actor::Context`]
    /// if the actor is restarted.
    inbox: Manager<NA::Message>,
    /// The running actor.
    actor: NA::Actor,
}

impl<S, NA> ActorProcess<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor,
    NA::RuntimeAccess: RuntimeSupport,
{
    /// Create a new `ActorProcess`.
    pub(crate) const fn new(
        supervisor: S,
        new_actor: NA,
        actor: NA::Actor,
        inbox: Manager<NA::Message>,
    ) -> ActorProcess<S, NA> {
        ActorProcess {
            supervisor,
            new_actor,
            inbox,
            actor,
        }
    }

    /// Returns `Ok(ProcessResult::Pending)` if the actor was successfully
    /// restarted, `Ok(ProcessResult::Complete)` if the actor wasn't restarted
    /// or an error if the actor failed to restart.
    fn handle_actor_error(
        &mut self,
        runtime_ref: &mut RuntimeRef,
        pid: ProcessId,
        err: <NA::Actor as Actor>::Error,
    ) -> ProcessResult {
        match self.supervisor.decide(err) {
            SupervisorStrategy::Restart(arg) => {
                match self.create_new_actor(runtime_ref, pid, arg) {
                    Ok(()) => {
                        // Mark the actor as ready just in case progress can be
                        // made already, this required because we use edge
                        // triggers for I/O.
                        NA::RuntimeAccess::mark_ready(runtime_ref, pid);
                        ProcessResult::Pending
                    }
                    Err(err) => self.handle_restart_error(runtime_ref, pid, err),
                }
            }
            SupervisorStrategy::Stop => ProcessResult::Complete,
        }
    }

    /// Same as `handle_actor_error` but handles [`NewActor::Error`]s instead.
    fn handle_restart_error(
        &mut self,
        runtime_ref: &mut RuntimeRef,
        pid: ProcessId,
        err: NA::Error,
    ) -> ProcessResult {
        match self.supervisor.decide_on_restart_error(err) {
            SupervisorStrategy::Restart(arg) => {
                match self.create_new_actor(runtime_ref, pid, arg) {
                    Ok(()) => {
                        // Mark the actor as ready, same reason as for
                        // `handle_actor_error`.
                        NA::RuntimeAccess::mark_ready(runtime_ref, pid);
                        ProcessResult::Pending
                    }
                    Err(err) => {
                        // Let the supervisor know.
                        self.supervisor.second_restart_error(err);
                        ProcessResult::Complete
                    }
                }
            }
            SupervisorStrategy::Stop => ProcessResult::Complete,
        }
    }

    /// Creates a new actor and, if successful, replaces the old actor with it.
    fn create_new_actor(
        &mut self,
        runtime_ref: &mut RuntimeRef,
        pid: ProcessId,
        arg: NA::Argument,
    ) -> Result<(), NA::Error> {
        let receiver = self.inbox.new_receiver().expect(
            "failed to create new receiver for actor's inbox. Was the `actor::Context` leaked?",
        );
        let ctx = NA::RuntimeAccess::new_context(pid, receiver, runtime_ref);
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
    NA: NewActor,
    NA::RuntimeAccess: rt::Access + RuntimeSupport,
{
    fn name(&self) -> &'static str {
        self.new_actor.name()
    }

    fn run(self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef, pid: ProcessId) -> ProcessResult {
        // This is safe because we're not moving the actor.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        // The actor need to be called with `Pin`. So we're undoing the previous
        // operation, still ensuring that the actor is not moved.
        let mut actor = unsafe { Pin::new_unchecked(&mut this.actor) };

        let waker = NA::RuntimeAccess::new_task_waker(runtime_ref, pid);
        let mut task_ctx = task::Context::from_waker(&waker);
        match actor.as_mut().try_poll(&mut task_ctx) {
            Poll::Ready(Ok(())) => ProcessResult::Complete,
            Poll::Ready(Err(err)) => this.handle_actor_error(runtime_ref, pid, err),
            Poll::Pending => ProcessResult::Pending,
        }
    }
}

/// Trait to support different kind of runtime access, e.g. [`ThreadSafe`] and
/// [`ThreadLocal`], within the same implementation of [`ActorProcess`].
pub(crate) trait RuntimeSupport {
    /// Creates a new context.
    fn new_context<M>(
        pid: ProcessId,
        inbox: Receiver<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> actor::Context<M, Self>
    where
        Self: Sized;

    /// Schedule the actor with `pid` for running (used after restart).
    fn mark_ready(runtime_ref: &mut RuntimeRef, pid: ProcessId);
}

impl RuntimeSupport for ThreadLocal {
    fn new_context<M>(
        pid: ProcessId,
        inbox: Receiver<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> actor::Context<M, ThreadLocal> {
        actor::Context::new(inbox, ThreadLocal::new(pid, runtime_ref.clone()))
    }

    fn mark_ready(runtime_ref: &mut RuntimeRef, pid: ProcessId) {
        runtime_ref.mark_ready_local(pid)
    }
}

impl RuntimeSupport for ThreadSafe {
    fn new_context<M>(
        pid: ProcessId,
        inbox: Receiver<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> actor::Context<M, ThreadSafe> {
        actor::Context::new(inbox, ThreadSafe::new(pid, runtime_ref.clone_shared()))
    }

    fn mark_ready(runtime_ref: &mut RuntimeRef, pid: ProcessId) {
        runtime_ref.mark_ready_shared(pid)
    }
}
