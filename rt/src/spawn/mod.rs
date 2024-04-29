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
//! The [`SpawnLocal`] trait defines how thread-local actors and futures are
//! spawn, while [`Spawn`] defines it for thread-safe variants.
//!
//! [`Actor`]: heph::actor::Actor
//! [`SyncActor`]: heph::sync::SyncActor
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
//! synchronisation. Futhermore these kind of actors are the cheapest to run
//! from a runtime perspective.
//!
//! The downside is that if a single actor blocks it will block *all* actors on
//! the thread the actor is running on. Something that some runtimes work around
//! with actor/futures/tasks that transparently move between threads and hide
//! blocking/bad actors, Heph does not (for thread-local actors).
//!
//! Thread-local actors and futures are spawned using [`SpawnLocal`].
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
//! Thread-safe actors and futures are spawned using [`SpawnLocal`].
//!
//! [`RuntimeRef::try_spawn`]: crate::RuntimeRef::try_spawn
//! [`ThreadSafe`]: crate::access::ThreadSafe

use std::future::Future;

use heph::future::ActorFutureBuilder;
use heph::supervisor::Supervisor;
use heph::{actor, ActorRef, NewActor};

use crate::access::{ThreadLocal, ThreadSafe};

pub mod options;

#[doc(no_inline)]
pub use options::{ActorOptions, FutureOptions, SyncActorOptions};

/// The `Spawn` trait defines how new thread-local [`Actor`]s and [`Future`]s
/// are added to the runtime.
///
/// [`Actor`]: heph::actor::Actor
pub trait SpawnLocal {
    /// Spawn a thread-local [`Future`].
    fn spawn_local_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + 'static;

    /// Attempt to spawn a thread-local actor.
    ///
    /// Arguments:
    /// * `supervisor`: all actors need supervision, the `supervisor` is the
    ///   supervisor for this actor, see the [`supervisor`] module for more
    ///   information.
    /// * `new_actor`: the [`NewActor`] implementation that defines how to start
    ///   the actor.
    /// * `arg`: the argument(s) passed when starting the actor, and
    /// * `options`: the actor options used to spawn the new actors.
    ///
    /// When using a [`NewActor`] implementation that never returns an error,
    /// such as the implementation provided by async functions, it's easier to
    /// use the [`spawn_local`] method.
    ///
    /// [`supervisor`]: heph::supervisor
    /// [`spawn_local`]: SpawnLocal::spawn_local
    fn try_spawn_local<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<RuntimeAccess = ThreadLocal> + 'static,
        ThreadLocal: for<'a> From<&'a Self> + 'static,
    {
        let (future, actor_ref) = ActorFutureBuilder::new()
            .with_rt(ThreadLocal::from(self))
            .with_inbox_size(options.inbox_size())
            .build(supervisor, new_actor, arg)?;
        self.spawn_local_future(future, options.into_future_options());
        Ok(actor_ref)
    }

    /// Spawn a thread-local actor.
    ///
    /// This is a convenience method for `NewActor` implementations that never
    /// return an error, such as asynchronous functions.
    ///
    /// See [`SpawnLocal::try_spawn_local`] for more information.
    fn spawn_local<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<NA::Message>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<Error = !, RuntimeAccess = ThreadLocal> + 'static,
        ThreadLocal: for<'a> From<&'a Self>,
    {
        match self.try_spawn_local(supervisor, new_actor, arg, options) {
            Ok(actor_ref) => actor_ref,
            Err(err) => err,
        }
    }
}

/// Thread-safe version of [`SpawnLocal`].
pub trait Spawn {
    /// Spawn a thread-safe [`Future`].
    fn spawn_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + Sync + Send + 'static;

    /// Attempt to spawn a thread-safe actor.
    ///
    /// See [`SpawnLocal::try_spawn_local`] for more information.
    fn try_spawn<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<RuntimeAccess = ThreadSafe> + Send + Sync + 'static,
        ThreadSafe: for<'a> From<&'a Self>,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        let (future, actor_ref) = ActorFutureBuilder::new()
            .with_rt(ThreadSafe::from(self))
            .with_inbox_size(options.inbox_size())
            .build(supervisor, new_actor, arg)?;
        self.spawn_future(future, options.into_future_options());
        Ok(actor_ref)
    }

    /// Spawn a thread-safe actor.
    ///
    /// This is a convenience method for `NewActor` implementations that never
    /// return an error, such as asynchronous functions.
    ///
    /// See [`Spawn::try_spawn`] for more information.
    fn spawn<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<NA::Message>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Error = !, RuntimeAccess = ThreadSafe> + Send + Sync + 'static,
        ThreadSafe: for<'a> From<&'a Self>,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        match self.try_spawn(supervisor, new_actor, arg, options) {
            Ok(actor_ref) => actor_ref,
            Err(err) => err,
        }
    }
}

impl<M, RT> SpawnLocal for actor::Context<M, RT>
where
    RT: SpawnLocal,
{
    fn spawn_local_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + 'static,
    {
        self.runtime().spawn_local_future(future, options);
    }
}

impl<M, RT> Spawn for actor::Context<M, RT>
where
    RT: Spawn,
{
    fn spawn_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + 'static + Send + Sync,
    {
        self.runtime().spawn_future(future, options);
    }
}
