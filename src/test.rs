//! Module with testing facilities.
//!
//! This module will lazily create an active, but not running, actor system per
//! thread. This means that for example an `ActorSystemRef` can be created and
//! used.
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

// The `new_count_waker` and related code is only used in testing, causing a
// dead_code warning. For now we'll have to allow it, I opened a PR to merge it
// into futures-test, a better place for it, see
// https://github.com/rust-lang-nursery/futures-rs/issues/1384.
#![allow(dead_code)]

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{LocalWaker, Wake, Poll, local_waker_from_nonlocal};

use crate::actor::{Actor, ActorContext, NewActor};
use crate::actor_ref::LocalActorRef;
use crate::mailbox::MailBox;
use crate::scheduler::ProcessId;
use crate::system::{ActorSystemRef, RunningActorSystem};
use crate::util::Shared;
use crate::waker::new_waker;

thread_local! {
    /// Per thread active, but not running, actor system.
    static TEST_SYSTEM: RefCell<RunningActorSystem> =
        RefCell::new(RunningActorSystem::new().unwrap());
}

/// Get a reference to the *test* actor system.
pub fn system_ref() -> ActorSystemRef {
    TEST_SYSTEM.with(|system| system.borrow().create_ref())
}

/// Initialise an actor.
pub fn init_actor<NA>(mut new_actor: NA, arg: NA::Argument) -> Result<(NA::Actor, LocalActorRef<NA::Message>), NA::Error>
    where NA: NewActor,
{
    let system_ref = system_ref();
    let pid = ProcessId(0);

    let inbox = Shared::new(MailBox::new(pid, system_ref.clone()));
    let actor_ref = LocalActorRef::new(inbox.downgrade());

    let ctx = ActorContext::new(pid, system_ref, inbox);
    let actor = new_actor.new(ctx, arg)?;

    Ok((actor, actor_ref))
}

/// Poll a future.
///
/// The `LocalWaker` be provided by the *test* actor system.
///
/// # Notes
///
/// Wake notifications will be ignored. If this is required run an end to end
/// test with a completely functional actor system instead.
pub fn poll_future<Fut>(future: Pin<&mut Fut>) -> Poll<Fut::Output>
    where Fut: Future,
{
    let waker = test_waker();
    Future::poll(future, &waker)
}

/// Poll an actor.
///
/// This is effectively the same function as [`poll_future`], but instead polls
/// an actors. The `LocalWaker` be provided by the *test* actor system.
///
/// # Notes
///
/// Wake notifications will be ignored. If this is required run an end to end
/// test with a completely functional actor system instead.
pub fn poll_actor<A>(actor: Pin<&mut A>) -> Poll<Result<(), A::Error>>
    where A: Actor,
{
    let waker = test_waker();
    Actor::try_poll(actor, &waker)
}

/// Create a test `LocalWaker`, with pid 0.
fn test_waker() -> LocalWaker {
    let pid = ProcessId(0);
    let mut system_ref = system_ref();
    let waker_notifications = system_ref.get_notification_sender();
    new_waker(pid, waker_notifications.clone())
}

/// Create a new `LocalWaker` that counts the number of times it's awoken.
pub(crate) fn new_count_waker() -> (LocalWaker, AwokenCount) {
    let inner = Arc::new(WakerInner { count: AtomicUsize::new(0) });
    (local_waker_from_nonlocal(inner.clone()), AwokenCount { inner })
}

/// Number of times the waker was awoken.
///
/// See [`new_count_waker`].
pub(crate) struct AwokenCount {
    inner: Arc<WakerInner>,
}

impl AwokenCount {
    /// Get the number of times the waker was awoken.
    pub fn get(&self) -> usize {
        self.inner.count.load(Ordering::SeqCst)
    }
}

struct WakerInner {
    count: AtomicUsize,
}

impl Wake for WakerInner {
    fn wake(arc_self: &Arc<Self>) {
        let _ = arc_self.count.fetch_add(1, Ordering::SeqCst);
    }
}
