//! The module with the supervisor, and related, types.
//!
//! # Supervisor
//!
//! A supervisor supervises an actor and handles its errors. A supervisor
//! generally does two things; logging and deciding whether to stop or restart
//! the actor. Its advised to keep a supervisor small and simple.
//!
//! When encountering an error it usually means someone has to be notified (to
//! fix it), something often done via logging.
//!
//! Next the supervisor needs to decide if the actor needs to be
//! [stopped](supervisor::SupervisorStrategy::Stop) or
//! [restarted](supervisor::SupervisorStrategy::Restart). If the supervisor decides to
//! restart the actor it needs to provide the argument to create a new actor
//! (used in calling the [`NewActor::new`](actor::NewActor::new) method).
//!
//! The restarted actor will have the same message inbox as the old (stopped)
//! actor. Note however that if an actor retrieved a message from its inbox, and
//! returned an error when processing it, the new (restarted) actor won't
//! retrieve that message again (messages aren't cloned after all).
//!
//! # Restarting or stopping?
//!
//! Sometimes just restarting an actor is the easiest way to deal with errors.
//! Starting the actor from a clean slate will often allow it to continue
//! processing. However this is not possible in all cases, for example when a
//! new argument can't be provided (think actors started by a
//! [`TcpListener`](net::TcpListener)). In those cases the supervisor should
//! still log the error encountered.
//!
//! # Examples
//!
//! Supervisor that logs the errors of a badly behaving actor and stops it.
//!
//! ```
//! #![feature(async_await, await_macro, futures_api, never_type)]
//!
//! use std::io;
//!
//! use heph::actor::ActorContext;
//! use heph::log::{self, error};
//! use heph::supervisor::SupervisorStrategy;
//! use heph::system::{ActorSystem, ActorOptions, RuntimeError};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     // Enable logging so we can see the error message.
//!     log::init();
//!
//!     ActorSystem::new().with_setup(|mut system_ref| {
//!         system_ref.spawn(supervisor, bad_actor as fn(_) -> _, (), ActorOptions {
//!             schedule: true,
//!             .. ActorOptions::default()
//!         });
//!         Ok(())
//!     })
//!     .run()
//! }
//!
//! /// The error returned by our actor.
//! struct Error;
//!
//! /// Supervisor that gets called if the actor returns an error.
//! fn supervisor(error: Error) -> SupervisorStrategy<()> {
//!     error!("Actor encountered an error!");
//!     SupervisorStrategy::Stop
//! }
//!
//! /// Our badly behaving actor.
//! async fn bad_actor(_ctx: ActorContext<!>) -> Result<(), Error> {
//!     Err(Error)
//! }
//! ```

/// The supervisor of an actor.
///
/// For more information about supervisors see the [module documentation], here
/// only the design of the trait is discussed.
///
/// The trait is designed to be generic to the error (`E`) and argument used in
/// restarting the actor (`Arg`). This means that the same type can implement
/// supervision for a number of different actors. But a word of caution,
/// supervisors should generally be small and simple, which means that having a
/// different supervisor for each actor is often a good thing.
///
/// `Supervisor` is implemented for any function that takes an error `E` and
/// returns `SupervisorStrategy<Arg>` automatically.
///
/// [module documentation]: index.html
pub trait Supervisor<E, Arg> {
    /// Decide what happens to the actor that returned `error`.
    fn decide(&mut self, error: E) -> SupervisorStrategy<Arg>;
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

impl<F, E, Arg> Supervisor<E, Arg> for F
    where F: FnMut(E) -> SupervisorStrategy<Arg>,
{
    fn decide(&mut self, error: E) -> SupervisorStrategy<Arg> {
        (self)(error)
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
/// #![feature(async_await, await_macro, futures_api, never_type)]
///
/// use std::io;
///
/// use heph::actor::ActorContext;
/// use heph::supervisor::NoSupervisor;
/// use heph::system::{ActorSystem, ActorOptions, RuntimeError};
///
/// fn main() -> Result<(), RuntimeError> {
///     ActorSystem::new().with_setup(|mut system_ref| {
///         system_ref.spawn(NoSupervisor, actor as fn(_) -> _, (), ActorOptions {
///             schedule: true,
///             .. ActorOptions::default()
///         });
///         Ok(())
///     })
///     .run()
/// }
///
/// /// Our actor that never returns an error.
/// async fn actor(mut ctx: ActorContext<&'static str>) -> Result<(), !> {
///     Ok(())
/// }
/// ```
#[derive(Copy, Clone, Debug)]
pub struct NoSupervisor;

impl<Arg> Supervisor<!, Arg> for NoSupervisor {
    fn decide(&mut self, _error: !) -> SupervisorStrategy<Arg> {
        // This can't be called.
        SupervisorStrategy::Stop
    }
}
