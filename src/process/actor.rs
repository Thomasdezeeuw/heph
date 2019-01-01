//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::fmt;
use std::pin::Pin;
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
    id: ProcessId,
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
    pub(crate) const fn new(id: ProcessId, supervisor: S, new_actor: NA, actor: NA::Actor, inbox: Shared<MailBox<NA::Message>>, waker: LocalWaker) -> ActorProcess<S, NA> {
        ActorProcess {
            id,
            supervisor,
            new_actor,
            inbox,
            actor,
            waker,
        }
    }
}

impl<S, NA> Process for ActorProcess<S, NA>
    where S: Supervisor<<NA::Actor as Actor>::Error, NA::Argument>,
          NA: NewActor + 'static,
{
    fn id(&self) -> ProcessId {
        self.id
    }

    fn run(self: Pin<&mut Self>, system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running actor process");

        // This is safe because we're not moving any values out.
        let this = unsafe { Pin::get_unchecked_mut(self) };

        // The actor need to be called with `Pin`. So we're undoing the previous
        // operation, still making sure that the actor is not moved.
        let pinned_actor = unsafe { Pin::new_unchecked(&mut this.actor) };
        let result = match Actor::try_poll(pinned_actor, &this.waker) {
            Poll::Ready(Ok(())) => ProcessResult::Complete,
            Poll::Ready(Err(err)) => {
                match this.supervisor.decide(err) {
                    SupervisorStrategy::Restart(arg) => {
                        // Create a new actor.
                        let ctx = ActorContext::new(this.id, system_ref.clone(), this.inbox.clone());
                        this.actor = this.new_actor.new(ctx, arg);
                        // Run the actor, just in case progress can be made
                        // already.
                        return unsafe { Pin::new_unchecked(this) }.run(system_ref);
                    },
                    SupervisorStrategy::Stop => ProcessResult::Complete,
                }
            },
            Poll::Pending => ProcessResult::Pending,
        };

        // Normally this should go in the `Drop` implementation, but we don't
        // have access to a system ref there, so we need to do it here.
        if let ProcessResult::Complete = result {
            system_ref.deregister::<NA>();
        }
        result
    }
}

impl<S, NA: NewActor> fmt::Debug for ActorProcess<S, NA> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorProcess")
            .field("id", &self.id)
            .finish()
    }
}
