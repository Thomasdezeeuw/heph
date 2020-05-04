//! The module with the supervisor, and related, types.
//!
//! # Supervisor
//!
//! A supervisor supervises an actor and handles any errors it encounters. A
//! supervisor generally does two things; logging of the error and deciding
//! whether to stop or restart the actor. Its advised to keep a supervisor small
//! and simple.
//!
//! When encountering an error it usually means someone has to be notified (to
//! fix it), something often done via logging.
//!
//! Next the supervisor needs to decide if the actor needs to be [stopped] or
//! [restarted]. If the supervisor decides to restart the actor it needs to
//! provide the argument to create a new actor (used in calling the
//! [`NewLocalActor::new`] method).
//!
//! The restarted actor will have the same message inbox as the old (stopped)
//! actor. Note however that if an actor retrieved a message from its inbox, and
//! returned an error when processing it, the new (restarted) actor won't
//! retrieve that message again (messages aren't cloned after all). If you want
//! to ensure that all messages are handled instead of receiving message they
// can be [peeked], which clones the message.
//! can be peeked, which clones the message.
//!
// [peeked]: crate::actor::LocalContext::peek
//!
//! # Restarting or stopping?
//!
//! Sometimes just restarting an actor is the easiest way to deal with errors.
//! Starting the actor from a clean slate will often allow it to continue
//! processing. However this is not possible in all cases, for example when a
//! new argument can't be provided (think actors started by a [`tcp::Server`]).
//! In those cases the supervisor should still log the error encountered.
//!
//! [stopped]: crate::supervisor::SupervisorStrategy::Stop
//! [restarted]: crate::supervisor::SupervisorStrategy::Restart
//! [`tcp::Server`]: crate::net::tcp::Server
//!
//! # Actors and sync actors
//!
//! As actors come in two flavours, [regular/asynchronous actors] and
//! [synchronous actors], thus so do the supervisor traits, [`Supervisor`] and
//! [`SyncSupervisor`].
//!
//! [regular/asynchronous actors]: crate::actor
//! [synchronous actors]: crate::actor::sync
//! [`Supervisor`]: crate::supervisor::Supervisor
//! [`SyncSupervisor`]: crate::supervisor::SyncSupervisor
//!
//! # Examples
//!
//! Supervisor that logs the errors of a badly behaving actor and stops it.
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::actor;
//! use heph::log::{self, error};
//! use heph::supervisor::SupervisorStrategy;
//! use heph::{ActorOptions, Runtime, RuntimeError};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     // Enable logging so we can see the error message.
//!     log::init();
//!
//!     Runtime::new()
//!         .with_setup(|mut runtime_ref| {
//!             runtime_ref.spawn_local(supervisor, bad_actor as fn(_) -> _, (),
//!                 ActorOptions::default().mark_ready());
//!             Ok(())
//!         })
//!         .start()
//! }
//!
//! /// The error returned by our actor.
//! struct Error;
//!
//! /// Supervisor that gets called if the actor returns an error.
//! fn supervisor(err: Error) -> SupervisorStrategy<()> {
//! #   drop(err); // Silence dead code warnings.
//!     error!("Actor encountered an error!");
//!     SupervisorStrategy::Stop
//! }
//!
//! /// Our badly behaving actor.
//! async fn bad_actor(_ctx: actor::LocalContext<!>) -> Result<(), Error> {
//!     Err(Error)
//! }
//! ```

use crate::actor::sync::SyncActor;
use crate::actor::{Actor, NewLocalActor};

/// The supervisor of an actor.
///
/// For more information about supervisors see the [module documentation], here
/// only the design of the trait is discussed.
///
/// The trait is designed to be generic over the [`NewLocalActor`]
/// implementation (`NA`). This means that the same type can implement
/// supervision for a number of different actors. But a word of caution,
/// supervisors should generally be small and simple, which means that having a
/// different supervisor for each actor is often a good thing.
///
/// `Supervisor` can be implemented using a simple function if the
/// `NewLocalActor` implementation doesn't return an error (i.e.
/// `NewLocalActor::Error = !`), which is the case for asynchronous functions.
/// See the [module documentation] for an example of this.
///
/// [module documentation]: crate::supervisor
pub trait Supervisor<NA>
where
    NA: NewLocalActor,
{
    /// Decide what happens to the actor that returned `error`.
    fn decide(&mut self, error: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument>;

    /// Decide what happens when an actor is restarted and the [`NewLocalActor`]
    /// implementation returns an `error`.
    fn decide_on_restart_error(&mut self, error: NA::Error) -> SupervisorStrategy<NA::Argument>;

    /// Method that gets call if an actor fails to restart twice.
    ///
    /// This is only called if [`decide`] returns a restart strategy, the actors
    /// fails to restart, after which [`decide_on_restart_error`] is called and
    /// also returns a restart strategy and restarting a second time also fails.
    /// We will not create an endless loop of restarting failures and instead
    /// call this function before stopping the actor (which can't be restarted
    /// any more).
    ///
    /// [`decide`]: Supervisor::decide
    /// [`decide_on_restart_error`]: Supervisor::decide_on_restart_error
    // TODO: a better name.
    fn second_restart_error(&mut self, error: NA::Error);
}

impl<F, NA> Supervisor<NA> for F
where
    F: FnMut(<NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument>,
    NA: NewLocalActor<Error = !>,
{
    fn decide(&mut self, err: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument> {
        (self)(err)
    }

    fn decide_on_restart_error(&mut self, _: !) -> SupervisorStrategy<NA::Argument> {
        // This can't be called.
        SupervisorStrategy::Stop
    }

    fn second_restart_error(&mut self, _: !) {
        // This can't be called.
    }
}

/// The strategy to use when handling an error from an actor.
///
/// See the [module documentation] for deciding on whether to restart an or not.
///
/// [module documentation]: index.html#restarting-or-stopping
#[derive(Debug)]
#[non_exhaustive]
pub enum SupervisorStrategy<Arg> {
    /// Restart the actor with the provided argument `Arg`.
    Restart(Arg),
    /// Stop the actor.
    Stop,
}

/// Supervisor for [synchronous actors].
///
/// For more information about supervisors see the [module documentation], here
/// only the design of the trait is discussed.
///
/// The trait is designed to be generic over the actor (`A`). This means that
/// the same type can implement supervision for a number of different actors.
/// But a word of caution, supervisors should generally be small and simple,
/// which means that having a different supervisor for each actor is often a
/// good thing.
///
/// `SyncSupervisor` is implemented for any function that takes an error `E` and
/// returns `SupervisorStrategy<Arg>` automatically.
///
/// [synchronous actors]: crate::actor::sync
/// [module documentation]: crate::supervisor
pub trait SyncSupervisor<A>
where
    A: SyncActor,
{
    /// Decide what happens to the actor that returned `error`.
    fn decide(&mut self, error: A::Error) -> SupervisorStrategy<A::Argument>;
}

impl<F, A> SyncSupervisor<A> for F
where
    F: FnMut(A::Error) -> SupervisorStrategy<A::Argument>,
    A: SyncActor,
{
    fn decide(&mut self, err: A::Error) -> SupervisorStrategy<A::Argument> {
        (self)(err)
    }
}

/// A supervisor implementation for actors that never return an error.
///
/// This supervisor does nothing and can't actually be called, it can only serve
/// as supervisor for actors that never return an error, i.e. actor that use the
/// never type (`!`) as error type.
///
/// # Example
///
/// ```
/// #![feature(never_type)]
///
/// use heph::supervisor::NoSupervisor;
/// use heph::{actor, RuntimeError, ActorOptions, Runtime};
///
/// fn main() -> Result<(), RuntimeError> {
///     Runtime::new()
///         .with_setup(|mut runtime_ref| {
///             runtime_ref.spawn_local(NoSupervisor, actor as fn(_) -> _, (),
///                 ActorOptions::default().mark_ready());
///             Ok(())
///         })
///         .start()
/// }
///
/// /// Our actor that never returns an error.
/// async fn actor(ctx: actor::LocalContext<&'static str>) -> Result<(), !> {
/// #   drop(ctx); // Silence dead code warnings.
///     Ok(())
/// }
/// ```
#[derive(Copy, Clone, Debug)]
pub struct NoSupervisor;

impl<NA, A> Supervisor<NA> for NoSupervisor
where
    NA: NewLocalActor<Actor = A, Error = !>,
    A: Actor<Error = !>,
{
    fn decide(&mut self, _: !) -> SupervisorStrategy<NA::Argument> {
        // This can't be called.
        SupervisorStrategy::Stop
    }

    fn decide_on_restart_error(&mut self, _: !) -> SupervisorStrategy<NA::Argument> {
        // This can't be called.
        SupervisorStrategy::Stop
    }

    fn second_restart_error(&mut self, _: !) {
        // This can't be called.
    }
}

impl<A> SyncSupervisor<A> for NoSupervisor
where
    A: SyncActor<Error = !>,
{
    fn decide(&mut self, _: !) -> SupervisorStrategy<A::Argument> {
        // This can't be called.
        SupervisorStrategy::Stop
    }
}
