//! Module containing the [`ActorFuture`].

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{self, Poll};

use heph_inbox::{Manager, ReceiverConnected};
use log::error;

use crate::actor::{self, Actor, NewActor};
use crate::actor_ref::ActorRef;
use crate::supervisor::{Supervisor, SupervisorStrategy};

/// A [`Future`] that represent an [`Actor`].
///
/// This can be used to wrap actors into a `Future`, it automatically handles
/// errors and panics using the provided supervisor. This can run on any runtime
/// that can runs `Future`s.
pub struct ActorFuture<S, NA: NewActor, RT> {
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
    /// Runtime access.
    rt: RT,
}

impl<S, NA, RT> ActorFuture<S, NA, RT>
where
    S: Supervisor<NA>,
    NA: NewActor<RuntimeAccess = RT>,
    RT: Clone,
{
    /// Create a new `ActorFuture`.
    ///
    /// Arguments:
    ///  * `supervisor: S`: is used to handle the actor's errors.
    ///  * `new_actor: NA`: is used to start the actor the first time and
    ///    restart it when it errors, for which the `argument` is used.
    ///  * `argument`: passed to the actor on creation.
    ///  * `rt: RT`: is used to get access to the runtime, it may be the unit
    ///    type (`()`) in case it's not needed. It needs to be `Clone` as it's
    ///    passed to the actor and after is needed for possible restarts.
    #[allow(clippy::type_complexity)]
    pub fn new(
        supervisor: S,
        mut new_actor: NA,
        argument: NA::Argument,
        rt: RT,
    ) -> Result<(ActorFuture<S, NA, RT>, ActorRef<NA::Message>), NA::Error> {
        let (inbox, sender, receiver) = heph_inbox::Manager::new_small_channel();
        let actor_ref = ActorRef::local(sender);
        let ctx = actor::Context::new(receiver, rt.clone());
        let actor = match new_actor.new(ctx, argument) {
            Ok(actor) => actor,
            Err(err) => return Err(err),
        };
        let future = ActorFuture {
            supervisor,
            new_actor,
            inbox,
            actor,
            rt,
        };
        Ok((future, actor_ref))
    }

    /// Returns the name of the actor.
    ///
    /// Based on the [`NewActor::name`] implementation.
    pub fn name() -> &'static str {
        NA::name()
    }

    #[doc(hidden)] // Not part of the stable API.
    pub fn pid(&self) -> usize {
        self.inbox.id().as_usize()
    }

    /// Returns `Poll::Pending` if the actor was successfully restarted,
    /// `Poll::Ready` if the actor wasn't restarted (or failed to restart).
    fn handle_actor_error(
        &mut self,
        waker: &task::Waker,
        err: <NA::Actor as Actor>::Error,
    ) -> Poll<()> {
        match self.supervisor.decide(err) {
            SupervisorStrategy::Restart(arg) => self.restart_actor(waker, arg),
            SupervisorStrategy::Stop => Poll::Ready(()),
        }
    }

    /// Returns `Poll::Pending` if the actor was successfully restarted,
    /// `Poll::Ready` if the actor wasn't restarted.
    fn handle_actor_panic(
        &mut self,
        waker: &task::Waker,
        panic: Box<dyn Any + Send + 'static>,
    ) -> Poll<()> {
        match self.supervisor.decide_on_panic(panic) {
            SupervisorStrategy::Restart(arg) => self.restart_actor(waker, arg),
            SupervisorStrategy::Stop => Poll::Ready(()),
        }
    }

    /// Attempt to restart the actor with `arg`.
    fn restart_actor(&mut self, waker: &task::Waker, arg: NA::Argument) -> Poll<()> {
        match self.create_new_actor(arg) {
            Ok(()) => {
                // Mark the actor as ready just in case progress can be made
                // already.
                waker.wake_by_ref();
                Poll::Pending
            }
            Err(err) => self.handle_restart_error(waker, err),
        }
    }

    /// Same as `handle_actor_error` but handles [`NewActor::Error`]s instead.
    fn handle_restart_error(&mut self, waker: &task::Waker, err: NA::Error) -> Poll<()> {
        match self.supervisor.decide_on_restart_error(err) {
            SupervisorStrategy::Restart(arg) => {
                match self.create_new_actor(arg) {
                    Ok(()) => {
                        // Mark the actor as ready, same reason as for
                        // `restart_actor`.
                        waker.wake_by_ref();
                        Poll::Pending
                    }
                    Err(err) => {
                        // Let the supervisor know.
                        self.supervisor.second_restart_error(err);
                        Poll::Ready(())
                    }
                }
            }
            SupervisorStrategy::Stop => Poll::Ready(()),
        }
    }

    /// Creates a new actor and, if successful, replaces the old actor with it.
    fn create_new_actor(&mut self, arg: NA::Argument) -> Result<(), NA::Error> {
        let receiver = self.inbox.new_receiver().unwrap_or_else(inbox_failure);
        let ctx = actor::Context::new(receiver, self.rt.clone());
        self.new_actor.new(ctx, arg).map(|actor| {
            // We pin the actor here to ensure its dropped in place when
            // replacing it with out new actor.
            unsafe { Pin::new_unchecked(&mut self.actor) }.set(actor);
        })
    }
}

impl<S, NA, RT> Future for ActorFuture<S, NA, RT>
where
    S: Supervisor<NA>,
    NA: NewActor<RuntimeAccess = RT>,
    RT: Clone,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving the actor.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        // SAFETY: undoing the previous operation, still ensuring that the actor
        // is not moved.
        let mut actor = unsafe { Pin::new_unchecked(&mut this.actor) };

        match catch_unwind(AssertUnwindSafe(|| actor.as_mut().try_poll(ctx))) {
            Ok(Poll::Ready(Ok(()))) => Poll::Ready(()),
            Ok(Poll::Ready(Err(err))) => this.handle_actor_error(ctx.waker(), err),
            Ok(Poll::Pending) => Poll::Pending,
            Err(panic) => {
                let msg = panic_message(&*panic);
                let name = NA::name();
                error!("actor '{name}' panicked at '{msg}'");
                this.handle_actor_panic(ctx.waker(), panic)
            }
        }
    }
}

impl<S, NA, RT> fmt::Debug for ActorFuture<S, NA, RT>
where
    S: Supervisor<NA> + fmt::Debug,
    NA: NewActor<RuntimeAccess = RT>,
    RT: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorFuture")
            .field("supervisor", &self.supervisor)
            .field("actor", &NA::name())
            .field("rt", &self.rt)
            .finish()
    }
}

/// Attempts to extract a message from a panic, defaulting to `<unknown>`.
/// NOTE: be sure to derefence the `Box`!
fn panic_message<'a>(panic: &'a (dyn Any + Send + 'static)) -> &'a str {
    match panic.downcast_ref::<&'static str>() {
        Some(s) => s,
        None => match panic.downcast_ref::<String>() {
            Some(s) => s,
            None => "<unknown>",
        },
    }
}

/// Called when we can't create a new receiver for the sync actor.
#[cold]
fn inbox_failure<T>(_: ReceiverConnected) -> T {
    panic!("failed to create new receiver for actor's inbox. Was the `actor::Context` leaked?");
}
