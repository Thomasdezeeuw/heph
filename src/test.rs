//! Testing facilities.
//!
//! This module will lazily create an active, but not running, runtime per
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
use std::cmp::max;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::{self, Poll};

use rand::Rng;

use crate::actor_ref::LocalActorRef;
use crate::inbox::{Inbox, InboxRef};
use crate::rt::worker::RunningRuntime;
use crate::rt::{ProcessId, Waker};
use crate::{actor, rt, Actor, NewActor, RuntimeRef};

thread_local! {
    /// Per thread active, but not running, runtime.
    static TEST_RT: RefCell<RunningRuntime> = {
        let (_, receiver) = rt::channel::new().unwrap();
        RefCell::new(RunningRuntime::new(receiver).unwrap())
    };
}

/// Returns a reference to the *test* runtime.
pub fn runtime() -> RuntimeRef {
    TEST_RT.with(|runtime| runtime.borrow().create_ref())
}

/// Initialise an actor.
#[allow(clippy::type_complexity)]
pub fn init_actor<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, LocalActorRef<NA::Message>), NA::Error>
where
    NA: NewActor,
{
    let runtime_ref = runtime();
    let pid = ProcessId(0);

    let waker = runtime_ref.new_waker(pid);
    let (inbox, inbox_ref) = Inbox::new(waker);
    let actor_ref = LocalActorRef::from_inbox(inbox_ref.clone());

    let ctx = actor::Context::new(pid, runtime_ref, inbox, inbox_ref);
    let actor = new_actor.new(ctx, arg)?;

    Ok((actor, actor_ref))
}

/// Initialise an actor.
///
/// Same as `init_actor`, but returns the inbox of the actor instead of an actor
/// reference.
#[allow(clippy::type_complexity)]
#[cfg(test)]
pub(crate) fn init_actor_inbox<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, Inbox<NA::Message>, InboxRef<NA::Message>), NA::Error>
where
    NA: NewActor,
{
    let runtime_ref = runtime();
    let pid = ProcessId(0);

    let waker = runtime_ref.new_waker(pid);
    let (inbox, inbox_ref) = Inbox::new(waker);

    let ctx = actor::Context::new(pid, runtime_ref, inbox.ctx_inbox(), inbox_ref.clone());
    let actor = new_actor.new(ctx, arg)?;

    Ok((actor, inbox, inbox_ref))
}

/// Poll a future.
///
/// The [`task::Context`] will be provided by the *test* runtime.
///
/// # Notes
///
/// Wake notifications will be ignored. If this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_future<Fut>(future: Pin<&mut Fut>) -> Poll<Fut::Output>
where
    Fut: Future,
{
    let pid = ProcessId(0);
    let waker = runtime().new_task_waker(pid);
    let mut ctx = task::Context::from_waker(&waker);
    Future::poll(future, &mut ctx)
}

/// Poll an actor.
///
/// This is effectively the same function as [`poll_future`], but instead polls
/// an actor. The [`task::Context`] will be provided by the *test* runtime.
///
/// # Notes
///
/// Wake notifications will be ignored. If this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_actor<A>(actor: Pin<&mut A>) -> Poll<Result<(), A::Error>>
where
    A: Actor,
{
    let pid = ProcessId(0);
    let waker = runtime().new_task_waker(pid);
    let mut ctx = task::Context::from_waker(&waker);
    Actor::try_poll(actor, &mut ctx)
}

/// Returns a new `Waker` for `pid`.
pub(crate) fn new_waker(pid: ProcessId) -> Waker {
    runtime().new_waker(pid)
}

/// Percentage of messages lost on purpose.
static MSG_LOSS: AtomicU8 = AtomicU8::new(0);

/// Set the percentage of messages lost on purpose.
///
/// This is useful to test the resilience of actors with respect to message
/// loss. Any and all messages send, thus including remote and local messages,
/// could be lost on purpose when using this function. Note that the send of the
/// messages will not return an error if the message is lost using this
/// function.
///
/// `percent` must be number between `0` and `100`, setting this to `0` (the
/// default) will disable the message loss.
pub fn set_message_loss(percent: u8) {
    let percent = max(percent, 100);
    MSG_LOSS.store(percent, Ordering::Release)
}

/// Returns `true` if the message should be lost.
pub(crate) fn should_lose_msg() -> bool {
    let loss = MSG_LOSS.load(Ordering::Relaxed);
    loss != 0 && rand::thread_rng().gen_range(0, 100) < loss
}
