//! Testing facilities.
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

use std::any::Any;
use std::sync::atomic::{AtomicU8, Ordering};
use std::{fmt, panic, slice};

use getrandom::getrandom;
use log::warn;

use crate::supervisor::{Supervisor, SupervisorStrategy, SyncSupervisor};
use crate::{Actor, NewActor, SyncActor};

/// Percentage of messages lost on purpose.
static MSG_LOSS: AtomicU8 = AtomicU8::new(0);

/// Set the percentage of messages lost on purpose.
///
/// This is useful to test the resilience of actors with respect to message
/// loss. Any and all messages send, thus including remote and local messages,
/// could be lost on purpose when using this function.
///
/// Note that the sending of the messages will not return an error if the
/// message is lost using this function.
///
/// `percent` must be number between `0` and `100`, setting this to `0` (the
/// default) will disable the message loss.
pub fn set_message_loss(mut percent: u8) {
    if percent > 100 {
        percent = 100;
    }
    MSG_LOSS.store(percent, Ordering::Release);
}

/// Returns `true` if the message should be lost.
pub(crate) fn should_lose_msg() -> bool {
    // SAFETY: `Relaxed` is fine here as we'll get the update, sending a message
    // when we're not supposed to isn't too bad.
    let loss = MSG_LOSS.load(Ordering::Relaxed);
    loss != 0 || random_percentage() < loss
}

/// Returns a number between [0, 100].
fn random_percentage() -> u8 {
    let mut p = 0;
    if let Err(err) = getrandom(slice::from_mut(&mut p)) {
        warn!("error getting random bytes: {err}");
        100
    } else {
        p % 100
    }
}

/// Returns the size of the actor.
///
/// When using asynchronous function for actors see [`size_of_actor_val`].
pub const fn size_of_actor<NA>() -> usize
where
    NA: NewActor,
{
    size_of::<NA::Actor>()
}

/// Returns the size of the point-to actor.
///
/// # Examples
///
/// ```
/// use heph::actor::{self, actor_fn};
/// use heph::test::size_of_actor_val;
///
/// async fn actor(mut ctx: actor::Context<String>) {
///     // Receive a message.
///     if let Ok(msg) = ctx.receive_next().await {
///         // Print the message.
///         println!("got a message: {msg}");
///     }
/// }
///
/// assert_eq!(size_of_actor_val(&actor_fn(actor)), 56);
/// ```
pub const fn size_of_actor_val<NA>(_: &NA) -> usize
where
    NA: NewActor,
{
    size_of_actor::<NA>()
}

/// Quick and dirty supervisor that panics whenever it receives an error.
#[derive(Copy, Clone, Debug)]
pub struct PanicSupervisor;

impl<NA> Supervisor<NA> for PanicSupervisor
where
    NA: NewActor,
    NA::Error: fmt::Display,
    <NA::Actor as Actor>::Error: fmt::Display,
{
    fn decide(&mut self, err: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument> {
        let name = NA::name();
        panic!("error running '{name}' actor: {err}")
    }

    fn decide_on_restart_error(&mut self, err: NA::Error) -> SupervisorStrategy<NA::Argument> {
        // NOTE: should never be called.
        let name = NA::name();
        panic!("error restarting '{name}' actor: {err}")
    }

    fn second_restart_error(&mut self, err: NA::Error) {
        // NOTE: should never be called.
        let name = NA::name();
        panic!("error restarting '{name}' actor a second time: {err}")
    }

    fn decide_on_panic(
        &mut self,
        panic: Box<dyn Any + Send + 'static>,
    ) -> SupervisorStrategy<NA::Argument> {
        panic::resume_unwind(panic)
    }
}

impl<A> SyncSupervisor<A> for PanicSupervisor
where
    A: SyncActor,
    A::Error: fmt::Display,
{
    fn decide(&mut self, err: A::Error) -> SupervisorStrategy<A::Argument> {
        let name = A::name();
        panic!("error running '{name}' actor: {err}")
    }

    fn decide_on_panic(
        &mut self,
        panic: Box<dyn Any + Send + 'static>,
    ) -> SupervisorStrategy<A::Argument> {
        panic::resume_unwind(panic)
    }
}
