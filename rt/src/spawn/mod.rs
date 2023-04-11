//! [`Spawn`]ing [`Actor`]s, [`SyncActor`]s and [`Future`]s.
//!
//! As discussed in the description of actors in [`heph::actor`], actors can
//! come in two flavours: asynchronous and synchronous. However asynchronous
//! actors and `Future`s can be run in two ways:
//!  * thread-local, and
//!  * thread-safe.
//!
//! The following sections describe the differences between thread-local and
//! thread-safe actors as well as the trade-offs between them. The sections only
//! talk of actors, but the same is true for `Future`s. We also have an example
//! of spawning actors in the [crate documentation].
//!
//! Quick advise: most actors and futures should run as thread-*local*
//! actors/futures. Unless they are known to block the runtime, in which case
//! they will cause less damage as thread-safe actors/futures, but this only
//! hides the issue, it doesn't solve it.
//!
//! [`Actor`]: heph::actor::Actor
//! [`SyncActor`]: heph::actor::SyncActor
//! [`Future`]: std::future::Future
//! [crate documentation]: crate#examples
//!
//! ## Asynchronous thread-local actors
//!
//! Asynchronous thread-local actors, often referred to as just thread-local
//! actors, are actors that will remain on the thread on which they are started.
//! They can be started, or spawned, using [`RuntimeRef::try_spawn_local`], or
//! any type that implements the [`Spawn`] trait using the [`ThreadLocal`]
//! context.
//!
//! The upside of running a thread-local actor is that it doesn't have to be
//! [`Send`] or [`Sync`], allowing it to use cheaper types that don't require
//! synchronisation. Futhermore these kind of actors are the cheapest to run.
//!
//! The downside is that if a single actor blocks it will block *all* actors on
//! the thread the actor is running on. Something that some runtimes work around
//! with actor/futures/tasks that transparently move between threads and hide
//! blocking/bad actors, Heph does not (for thread-local actor).
//!
//! [`RuntimeRef::try_spawn_local`]: crate::RuntimeRef::try_spawn_local
//! [`ThreadLocal`]: crate::access::ThreadLocal
//!
//! ## Asynchronous thread-safe actors
//!
//! Asynchronous thread-safe actors, or just thread-safe actors, are actors that
//! can be run on any of the worker threads and transparently move between them.
//! They can be spawned using [`RuntimeRef::try_spawn`], or any type that
//! implements the [`Spawn`] trait using the [`ThreadSafe`] context. Because
//! these actors move between threads they are required to be [`Send`] and
//! [`Sync`].
//!
//! An upside to using thread-safe actors is that a bad actor (that blocks) only
//! blocks a single worker thread at a time, allowing the other worker threads
//! to run the other thread-safe actors (but not the thread-local actors!).
//!
//! A downside is that these actors are more expansive to run than thread-local
//! actors.
//!
//! [`RuntimeRef::try_spawn`]: crate::RuntimeRef::try_spawn
//! [`ThreadSafe`]: crate::access::ThreadSafe

use heph::supervisor::Supervisor;
use heph::{actor, ActorRef, NewActor};

pub mod options;

pub(crate) use private::{AddActorError, PrivateSpawn};

#[doc(no_inline)]
pub use options::{ActorOptions, FutureOptions, SyncActorOptions};

/// The `Spawn` trait defines how new actors are added to the runtime.
pub trait Spawn<S, NA, RT>: PrivateSpawn<S, NA, RT> {
    /// Attempts to spawn an actor.
    ///
    /// Arguments:
    /// * `supervisor`: all actors need supervision, the `supervisor` is the
    ///   supervisor for this actor, see the [`Supervisor`] trait for more
    ///   information.
    /// * `new_actor`: the [`NewActor`] implementation that defines how to start
    ///   the actor.
    /// * `arg`: the argument(s) passed when starting the actor, and
    /// * `options`: the actor options used to spawn the new actors.
    ///
    /// When using a [`NewActor`] implementation that never returns an error,
    /// such as the implementation provided by async functions, it's easier to
    /// use the [`spawn`] method.
    ///
    /// [`spawn`]: Spawn::spawn
    fn try_spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = RT>,
    {
        self.try_spawn_setup(supervisor, new_actor, |_| Ok(arg), options)
            .map_err(|err| match err {
                AddActorError::NewActor(err) => err,
                AddActorError::<_, !>::ArgFn(_) => unreachable!(),
            })
    }

    /// Spawn an actor.
    ///
    /// This is a convenience method for `NewActor` implementations that never
    /// return an error, such as asynchronous functions.
    ///
    /// See [`Spawn::try_spawn`] for more information.
    fn spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<NA::Message>
    where
        S: Supervisor<NA>,
        NA: NewActor<Error = !, RuntimeAccess = RT>,
    {
        match self.try_spawn(supervisor, new_actor, arg, options) {
            Ok(actor_ref) => actor_ref,
            Err(err) => err,
        }
    }
}

mod private {
    //! Module with private types.

    use heph::actor::{self, NewActor};
    use heph::actor_ref::ActorRef;
    use heph::supervisor::Supervisor;

    use crate::spawn::ActorOptions;

    /// Private version of the [`Spawn`]  trait.
    ///
    /// [`Spawn`]: super::Spawn
    pub trait PrivateSpawn<S, NA, RT> {
        /// Spawn an actor that needs to be initialised.
        ///
        /// See the public [`Spawn`] trait for documentation on the arguments.
        ///
        /// [`Spawn`]: super::Spawn
        #[allow(clippy::type_complexity)] // Not part of the public API, so it's OK.
        fn try_spawn_setup<ArgFn, E>(
            &mut self,
            supervisor: S,
            new_actor: NA,
            arg_fn: ArgFn,
            options: ActorOptions,
        ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, E>>
        where
            S: Supervisor<NA>,
            NA: NewActor<RuntimeAccess = RT>,
            ArgFn: FnOnce(&mut actor::Context<NA::Message, RT>) -> Result<NA::Argument, E>;
    }

    /// Error returned by spawning a actor.
    #[derive(Debug)]
    pub enum AddActorError<NewActorE, ArgFnE> {
        /// Calling `NewActor::new` actor resulted in an error.
        NewActor(NewActorE),
        /// Calling the argument function resulted in an error.
        ArgFn(ArgFnE),
    }
}

impl<M, RT, S, NA, RT2> Spawn<S, NA, RT2> for actor::Context<M, RT> where RT: Spawn<S, NA, RT2> {}

impl<M, RT, S, NA, RT2> PrivateSpawn<S, NA, RT2> for actor::Context<M, RT>
where
    RT: PrivateSpawn<S, NA, RT2>,
{
    fn try_spawn_setup<ArgFn, E>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, E>>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = RT2>,
        ArgFn: FnOnce(&mut actor::Context<NA::Message, RT2>) -> Result<NA::Argument, E>,
    {
        self.runtime()
            .try_spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}
