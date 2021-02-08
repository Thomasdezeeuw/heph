//! Module containing the implementation of the [`Process`] trait for
//! [`Actor`]s.

use std::pin::Pin;
use std::task::{self, Poll};

use inbox::{Manager, Receiver};

use crate::actor::context::{ThreadLocal, ThreadSafe};
use crate::actor::{self, Actor, NewActor};
use crate::rt::process::{Process, ProcessId, ProcessResult};
use crate::supervisor::SupervisorStrategy;
use crate::{RuntimeRef, Supervisor};

/// A process that represent an [`Actor`].
pub(in crate::rt) struct ActorProcess<S, NA: NewActor> {
    /// The actor's supervisor used to determine what to do when the actor, or
    /// [`NewActor`] implementation, returns an error.
    supervisor: S,
    /// The [`NewActor`] implementation used to restart the actor.
    new_actor: NA,
    /// The inbox of the actor, used in creating a new [`actor::Context`]
    /// if the actor is restarted.
    inbox: Manager<NA::Message>,
    /// The running actors.
    actor: NA::Actor,
}

impl<'a, S, NA: NewActor, C> ActorProcess<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor<Context = C>,
    C: ContextKind,
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
            actor,
            inbox,
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
    ) -> Result<ProcessResult, NA::Error> {
        match self.supervisor.decide(err) {
            SupervisorStrategy::Restart(arg) => self
                .create_new_actor(runtime_ref, pid, arg)
                .map(|()| ProcessResult::Pending),
            SupervisorStrategy::Stop => Ok(ProcessResult::Complete),
        }
    }

    /// Same as `handle_actor_error` but handles [`NewActor::Error`]s instead.
    fn handle_restart_error(
        &mut self,
        runtime_ref: &mut RuntimeRef,
        pid: ProcessId,
        err: NA::Error,
    ) -> Result<ProcessResult, NA::Error> {
        match self.supervisor.decide_on_restart_error(err) {
            SupervisorStrategy::Restart(arg) => self
                .create_new_actor(runtime_ref, pid, arg)
                .map(|()| ProcessResult::Pending),
            SupervisorStrategy::Stop => Ok(ProcessResult::Complete),
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
        let ctx = C::new_context(pid, receiver, runtime_ref);
        self.new_actor.new(ctx, arg).map(|actor| {
            // We pin the actor here to ensure its dropped in place when
            // replacing it with out new actor.
            unsafe { Pin::new_unchecked(&mut self.actor) }.set(actor)
        })
    }
}

impl<'a, S, NA, C> Process for ActorProcess<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor<Context = C>,
    C: ContextKind,
{
    fn name(&self) -> &'static str {
        actor::name::<NA::Actor>()
    }

    fn run(self: Pin<&mut Self>, runtime_ref: &mut RuntimeRef, pid: ProcessId) -> ProcessResult {
        // This is safe because we're not moving the actor.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        // The actor need to be called with `Pin`. So we're undoing the previous
        // operation, still ensuring that the actor is not moved.
        let mut actor = unsafe { Pin::new_unchecked(&mut this.actor) };

        let waker = C::new_task_waker(runtime_ref, pid);
        let mut task_ctx = task::Context::from_waker(&waker);
        match actor.as_mut().try_poll(&mut task_ctx) {
            Poll::Ready(Ok(())) => ProcessResult::Complete,
            Poll::Ready(Err(err)) => match this.handle_actor_error(runtime_ref, pid, err) {
                Ok(ProcessResult::Pending) => {
                    // Run the actor just in case progress can be made already,
                    // this required because we use edge triggers for I/O.
                    unsafe { Pin::new_unchecked(this) }.run(runtime_ref, pid)
                }
                // Actor wasn't restarted.
                Ok(ProcessResult::Complete) => ProcessResult::Complete,
                Err(err) => match this.handle_restart_error(runtime_ref, pid, err) {
                    Ok(ProcessResult::Pending) => {
                        // Run the actor, same reason as above.
                        unsafe { Pin::new_unchecked(this) }.run(runtime_ref, pid)
                    }
                    // Actor wasn't restarted.
                    Ok(ProcessResult::Complete) => ProcessResult::Complete,
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

/// Support all kinds of actor context's (e.g. [`ThreadSafe`] and
/// [`ThreadLocal`]) within the same implementation of [`ActorProcess`].
pub(in crate::rt) trait ContextKind {
    /// Create a new [`task::Waker`].
    fn new_task_waker(runtime_ref: &mut RuntimeRef, pid: ProcessId) -> task::Waker;

    /// Creates a new context.
    fn new_context<M>(
        pid: ProcessId,
        inbox: Receiver<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> actor::Context<M, Self>
    where
        Self: Sized;
}

impl ContextKind for ThreadLocal {
    fn new_task_waker(runtime_ref: &mut RuntimeRef, pid: ProcessId) -> task::Waker {
        runtime_ref.new_local_task_waker(pid)
    }

    fn new_context<M>(
        pid: ProcessId,
        inbox: Receiver<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> actor::Context<M, ThreadLocal> {
        actor::Context::new_local(pid, inbox, runtime_ref.clone())
    }
}

impl ContextKind for ThreadSafe {
    fn new_task_waker(runtime_ref: &mut RuntimeRef, pid: ProcessId) -> task::Waker {
        runtime_ref.new_shared_task_waker(pid)
    }

    fn new_context<M>(
        pid: ProcessId,
        inbox: Receiver<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> actor::Context<M, ThreadSafe> {
        actor::Context::new_shared(pid, inbox, runtime_ref.clone_shared())
    }
}
