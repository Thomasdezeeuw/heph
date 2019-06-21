//! Testing facilities.
//!
//! This module will lazily create an active, but not running, actor system per
//! thread.
//!
//! # Notes
//!
//! *This module is only available when the `test` feature is enabled*. It
//! shouldn't be enabled by default, and shouldn't end up in your production
//! binary.
//!
//! It is possible to only enable the test feature when testing by adding the
//! following to `Cargo.toml`.
//!
//! ```toml
//! [dev-dependencies.heph]
//! features = ["test"]
//! ```

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::{self, Poll};

use crate::actor_ref::{ActorRef, Local};
use crate::mailbox::MailBox;
use crate::system::scheduler::ProcessId;
use crate::system::RunningActorSystem;
use crate::util::Shared;
use crate::{actor, Actor, ActorSystemRef, NewActor};

thread_local! {
    /// Per thread active, but not running, actor system.
    static TEST_SYSTEM: RefCell<RunningActorSystem> =
        RefCell::new(RunningActorSystem::new::<!>().unwrap());
}

/// Get a reference to the *test* actor system.
pub fn system_ref() -> ActorSystemRef {
    TEST_SYSTEM.with(|system| system.borrow().create_ref())
}

/// Initialise an actor.
#[allow(clippy::type_complexity)]
pub fn init_actor<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, ActorRef<Local<NA::Message>>), NA::Error>
where
    NA: NewActor,
{
    let system_ref = system_ref();
    let pid = ProcessId(0);

    let inbox = Shared::new(MailBox::new(pid, system_ref.clone()));
    let actor_ref = ActorRef::new_local(inbox.downgrade());

    let ctx = actor::Context::new(pid, system_ref, inbox);
    let actor = new_actor.new(ctx, arg)?;

    Ok((actor, actor_ref))
}

/// Poll a future.
///
/// The [`task::Context`] will be provided by the *test* actor system.
///
/// # Notes
///
/// Wake notifications will be ignored. If this is required run an end to end
/// test with a completely functional actor system instead.
pub fn poll_future<Fut>(future: Pin<&mut Fut>) -> Poll<Fut::Output>
where
    Fut: Future,
{
    let pid = ProcessId(0);
    let waker = system_ref().new_waker(pid);
    let mut ctx = task::Context::from_waker(&waker);
    Future::poll(future, &mut ctx)
}

/// Poll an actor.
///
/// This is effectively the same function as [`poll_future`], but instead polls
/// an actor. The [`task::Context`] will be provided by the *test* actor system.
///
/// # Notes
///
/// Wake notifications will be ignored. If this is required run an end to end
/// test with a completely functional actor system instead.
pub fn poll_actor<A>(actor: Pin<&mut A>) -> Poll<Result<(), A::Error>>
where
    A: Actor,
{
    let pid = ProcessId(0);
    let waker = system_ref().new_waker(pid);
    let mut ctx = task::Context::from_waker(&waker);
    Actor::try_poll(actor, &mut ctx)
}
