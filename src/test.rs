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
//! TODO: doc how to enable the features only in development/testing.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use crate::actor::{ActorContext, NewActor};
use crate::actor_ref::LocalActorRef;
use crate::mailbox::MailBox;
use crate::process::ProcessId;
use crate::system::{ActorSystemRef, RunningActorSystem};
use crate::util::Shared;
use crate::waker::new_waker;

thread_local! {
    /// Per thread active, but not running, actor system.
    static TEST_SYSTEM: RefCell<RunningActorSystem> =
        RefCell::new(RunningActorSystem::new().unwrap());
}

/// Get a reference to the testing actor system.
pub fn system_ref() -> ActorSystemRef {
    TEST_SYSTEM.with(|system| system.borrow().create_ref())
}

/// Initialise an actor.
pub fn init_actor<N>(mut new_actor: N, item: N::StartItem) -> (N::Actor, LocalActorRef<N::Message>)
    where N: NewActor,
{
    let system_ref = system_ref();
    let pid = ProcessId(0);

    let inbox = Shared::new(MailBox::new(pid, system_ref.clone()));
    let actor_ref = LocalActorRef::new(inbox.downgrade());

    let ctx = ActorContext::new(pid, system_ref, inbox);
    let actor = new_actor.new(ctx, item);

    (actor, actor_ref)
}

/// Poll a future, letting the task `Context` be provided by the actor system.
///
/// # Notes
///
/// Wake notifications will be ignored. If this is required run an end to end
/// test with a completely functional actor system instead.
pub fn poll_future<Fut>(future: Pin<&mut Fut>) -> Poll<Fut::Output>
    where Fut: Future,
{
    let pid = ProcessId(0);
    let mut system_ref = system_ref();
    let waker_notifications = system_ref.get_notification_sender();
    let waker = new_waker(pid, waker_notifications.clone());
    Future::poll(future, &waker)
}
