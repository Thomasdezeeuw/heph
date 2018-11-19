//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::{fmt, ptr};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::process::abort;
use std::task::{LocalWaker, Poll};

use log::trace;

use crate::actor::{Actor, NewActor, ActorContext};
use crate::mailbox::MailBox;
use crate::process::{Process, ProcessId, ProcessResult};
use crate::supervisor::{Supervisor, SupervisorStrategy};
use crate::system::ActorSystemRef;
use crate::util::Shared;

/// A process that represent an [`Actor`].
///
/// [`Actor`]: ../../actor/trait.Actor.html
pub struct ActorProcess<S, NA: NewActor> {
    /// The id of this process.
    pid: ProcessId,
    /// Supervisor of the actor.
    supervisor: S,
    /// `NewActor` used to restart the actor.
    new_actor: NA,
    /// The actor.
    actor: NA::Actor,
    /// The inbox of the actor, used in create a new `ActorContext` if the actor
    /// is restarted.
    inbox: Shared<MailBox<NA::Message>>,
    /// Waker used in the futures context.
    waker: LocalWaker,
}

impl<S, NA: NewActor> ActorProcess<S, NA> {
    /// Create a new `ActorProcess`.
    pub(crate) fn new(pid: ProcessId, supervisor: S, new_actor: NA, actor: NA::Actor, inbox: Shared<MailBox<NA::Message>>, waker: LocalWaker) -> ActorProcess<S, NA> {
        ActorProcess {
            pid,
            supervisor,
            new_actor,
            inbox,
            actor,
            waker,
        }
    }

    // Replace the actor, dropping the old one.
    fn swap_actor(&mut self, actor: NA::Actor) {
        // We can't call `drop(mem::replace(&mut sel.actor, actor));`, because
        // that would move the actor to stack before dropping it. That is not
        // allowed when pinned, so we need to drop the actor in place. However
        // when the actor's Drop implementation panics it could be that that the
        // actor is dropped again when `ActorProcess` is dropped, causing a
        // double free. So we need to catch any panics and abort, as that is the
        // only safe option left. If we don't have any panics we can just
        // overwrite the old actor.
        let res = catch_unwind(AssertUnwindSafe(|| {
            unsafe { ptr::drop_in_place(&mut self.actor); }
        }));

        match res {
            Ok(()) => unsafe { ptr::write(&mut self.actor, actor); },
            Err(_) => abort(),
        }
    }
}

impl<S, NA> Process for ActorProcess<S, NA>
    where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument>,
          NA: NewActor,
{
    fn run(self: Pin<&mut Self>, system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running actor process");

        // The only thing in `ActorProcess` that is `!Unpin` is the actor. So
        // this is safe as long as we don't move the actor.
        let this = unsafe { Pin::get_mut_unchecked(self) };

        // The actor does need to be called with `Pin`. So we're undoing the
        // previous operation, still making sure that the actor is not moved.
        let pinned_actor = unsafe { Pin::new_unchecked(&mut this.actor) };
        match pinned_actor.try_poll(&this.waker) {
            Poll::Ready(Ok(())) => ProcessResult::Complete,
            Poll::Ready(Err(err)) => {
                match this.supervisor.decide(err) {
                    SupervisorStrategy::Restart(arg) => {
                        // Create a new actor.
                        let ctx = ActorContext::new(this.pid, system_ref.clone(), this.inbox.clone());
                        let actor = this.new_actor.new(ctx, arg);
                        // Swap the old actor with the new one.
                        this.swap_actor(actor);
                        // Run the actor, just in case progress can be made
                        // already.
                        return unsafe { Pin::new_unchecked(this) }.run(system_ref);
                    },
                    SupervisorStrategy::Stop => ProcessResult::Complete,
                }
            },
            Poll::Pending => ProcessResult::Pending,
        }
    }
}

impl<S, NA: NewActor> fmt::Debug for ActorProcess<S, NA> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorProcess")
            .finish()
    }
}
