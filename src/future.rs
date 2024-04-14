//! The [`ActorFuture`] and related types.

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::num::NonZeroU8;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::task::{self, Poll};

use heph_inbox::{self as inbox, ReceiverConnected};
use log::error;

use crate::actor::{self, Actor, NewActor};
use crate::actor_ref::ActorRef;
use crate::panic_message;
use crate::supervisor::{Supervisor, SupervisorStrategy};

/// A [`Future`] that represent an [`Actor`].
///
/// This can be used to wrap actors into a `Future`, it automatically handles
/// errors and panics using the provided supervisor. This can run on any runtime
/// that can run `Future`s.
///
/// See [`ActorFutureBuilder`] for setting various options.
pub struct ActorFuture<S, NA: NewActor> {
    /// The actor's supervisor used to determine what to do when the actor, or
    /// the [`NewActor`] implementation, returns an error or panics.
    supervisor: S,
    /// The [`NewActor`] implementation used to restart the actor.
    new_actor: NA,
    /// The inbox of the actor, used in creating a new [`actor::Context`]
    /// if the actor is restarted.
    inbox: inbox::Manager<NA::Message>,
    /// The running actor.
    actor: NA::Actor,
    /// Runtime access.
    rt: NA::RuntimeAccess,
}

impl<S, NA> ActorFuture<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor,
    NA::RuntimeAccess: Clone,
{
    /// Create a new `ActorFuture`.
    ///
    /// Arguments:
    ///  * `supervisor: S`: is used to handle the actor's errors.
    ///  * `new_actor: NA`: is used to start the actor the first time and
    ///    restart it when it errors, for which the `argument` is used.
    ///  * `argument`: passed to the actor on creation.
    ///
    /// For creation of an [`ActorFuture`] with non-default options see
    /// [`ActorFutureBuilder`].
    #[allow(clippy::type_complexity)]
    pub fn new(
        supervisor: S,
        new_actor: NA,
        argument: NA::Argument,
    ) -> Result<(ActorFuture<S, NA>, ActorRef<NA::Message>), NA::Error>
    where
        NA: NewActor<RuntimeAccess = ()>,
    {
        ActorFutureBuilder::new().build(supervisor, new_actor, argument)
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

impl<S, NA> Future for ActorFuture<S, NA>
where
    S: Supervisor<NA>,
    NA: NewActor,
    NA::RuntimeAccess: Clone,
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

#[allow(clippy::missing_fields_in_debug)]
impl<S, NA> fmt::Debug for ActorFuture<S, NA>
where
    S: Supervisor<NA> + fmt::Debug,
    NA: NewActor,
    NA::RuntimeAccess: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorFuture")
            .field("supervisor", &self.supervisor)
            .field("actor", &NA::name())
            .field("inbox", &self.inbox)
            .field("rt", &self.rt)
            .finish()
    }
}

/// Called when we can't create a new receiver for the sync actor.
#[cold]
fn inbox_failure<T>(_: ReceiverConnected) -> T {
    panic!("failed to create new receiver for actor's inbox. Was the `actor::Context` leaked?");
}

/// Builder for [`ActorFuture`].
///
/// This allows setting various options.
#[derive(Debug)]
#[must_use = "call `build` to finish building the `ActorFuture`"]
pub struct ActorFutureBuilder<RT = ()> {
    rt: RT,
    inbox_size: InboxSize,
}

impl ActorFutureBuilder {
    /// Create a new `ActorFutureBuilder`, which allows for the creation of
    /// `ActorFuture` with more options.
    pub const fn new() -> ActorFutureBuilder {
        ActorFutureBuilder {
            rt: (),
            inbox_size: InboxSize::DEFAULT,
        }
    }
}

impl<RT> ActorFutureBuilder<RT> {
    /// Returns the runtime access used by the actor.
    pub fn rt(&self) -> &RT {
        &self.rt
    }

    /// Set the runtime access used by the actor.
    ///
    /// `rt: RT`: is used to get access to the runtime, it defaults to the unit
    /// type (`()`) in case it's not needed. It needs to be `Clone` as it's
    /// passed to the actor and after is needed for possible restarts.
    pub fn with_rt<RT2>(self, rt: RT2) -> ActorFutureBuilder<RT2>
    where
        RT: Clone,
    {
        ActorFutureBuilder {
            rt,
            inbox_size: self.inbox_size,
        }
    }

    /// Returns the size of the actor's inbox.
    pub fn inbox_size(&self) -> InboxSize {
        self.inbox_size
    }

    /// Set the size of the actor's inbox.
    pub fn with_inbox_size(mut self, size: InboxSize) -> Self {
        self.inbox_size = size;
        self
    }

    /// Create a new `ActorFuture`.
    ///
    /// Arguments:
    ///  * `supervisor: S`: is used to handle the actor's errors.
    ///  * `new_actor: NA`: is used to start the actor the first time and
    ///    restart it when it errors, for which the `argument` is used.
    ///  * `argument`: passed to the actor on creation.
    #[allow(clippy::type_complexity)]
    pub fn build<S, NA>(
        self,
        supervisor: S,
        mut new_actor: NA,
        argument: NA::Argument,
    ) -> Result<(ActorFuture<S, NA>, ActorRef<NA::Message>), NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = RT>,
        RT: Clone,
    {
        let rt = self.rt;
        let (inbox, sender, receiver) = inbox::Manager::new_channel(self.inbox_size.get());
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
}

/// Maximum size of the actor's inbox before sending to it will (asynchronously)
/// block the sender.
///
/// # Notes
///
/// The sizes defines by the constants may change in the future.
#[derive(Copy, Clone, Debug)]
pub struct InboxSize(NonZeroU8);

impl InboxSize {
    /// Space for a single message.
    pub const ONE: InboxSize = InboxSize(NonZeroU8::new(1).unwrap());

    /// Small inbox, currently 8 messages.
    pub const SMALL: InboxSize = InboxSize(NonZeroU8::new(8).unwrap());

    /// Medium sized inbox, currently 16 messages.
    pub const MEDIUM: InboxSize = InboxSize(NonZeroU8::new(16).unwrap());

    /// Maximum inbox size, currently 24 messages.
    pub const LARGE: InboxSize = InboxSize(NonZeroU8::new(24).unwrap());

    /// Maximum inbox size, currently 29 messages.
    #[allow(clippy::cast_possible_truncation)]
    pub const MAX: InboxSize = InboxSize(NonZeroU8::new(heph_inbox::MAX_CAP as u8).unwrap());

    /// Default inbox size.
    pub(crate) const DEFAULT: InboxSize = InboxSize::SMALL;

    /// Use a fixed inbox size.
    ///
    /// Returns an error if the inbox size is either zero or too big.
    #[allow(clippy::cast_possible_truncation)] // We check the capacity.
    pub const fn fixed(size: usize) -> Result<InboxSize, InvalidInboxSize> {
        assert!(heph_inbox::MAX_CAP <= u8::MAX as usize);
        if size >= 1 && size <= heph_inbox::MAX_CAP {
            Ok(InboxSize(NonZeroU8::new(size as u8).unwrap()))
        } else {
            Err(InvalidInboxSize)
        }
    }

    /// Returns itself as `usize`.
    pub const fn get(self) -> usize {
        // TODO: replace with `usize::from`, once const stable.
        self.0.get() as usize
    }
}

impl Default for InboxSize {
    fn default() -> InboxSize {
        InboxSize::DEFAULT
    }
}

/// Error returned by [`InboxSize::fixed`].
#[derive(Debug)]
#[non_exhaustive]
pub struct InvalidInboxSize;

impl fmt::Display for InvalidInboxSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "actor inbox capacity must be between {} and {}",
            heph_inbox::MIN_CAP,
            heph_inbox::MAX_CAP
        )
    }
}

impl Error for InvalidInboxSize {}
