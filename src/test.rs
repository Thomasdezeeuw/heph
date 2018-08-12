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

use crate::actor::{ActorContext, NewActor};
use crate::actor_ref::LocalActorRef;
use crate::mailbox::MailBox;
use crate::process::ProcessId;
use crate::system::{ActorSystemRef, RunningActorSystem};
use crate::util::Shared;

thread_local! {
    /// Per thread active, but not running, actor system.
    static TEST_SYSTEM: RefCell<RunningActorSystem> =
        RefCell::new(RunningActorSystem::new().unwrap());
}

/// Get a reference to the testing actor system.
pub fn system_ref() -> ActorSystemRef {
    TEST_SYSTEM.with(|system| {
        system.borrow().create_ref()
    })
}

/// Initialise an actor.
pub fn init_actor<N>(mut new_actor: N, item: N::Item) -> (N::Actor, LocalActorRef<N::Message>)
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

// TODO: provide a way to run actor, or provide a `task::Context`.
