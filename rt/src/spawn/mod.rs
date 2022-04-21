//! Module with the [`Spawn`] trait.

use heph::actor::{self, NewActor};
use heph::actor_ref::ActorRef;
use heph::supervisor::Supervisor;

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
        self.try_spawn_setup(supervisor, new_actor, |_| Ok(arg), options)
            .unwrap_or_else(|_: AddActorError<!, !>| unreachable!())
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
