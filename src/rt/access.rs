//! Module to that defines the [`rt::Access`] trait.
//!
//! [`rt::Access`]: crate::rt::Access

use std::ops::{Deref, DerefMut};
use std::time::Instant;
use std::{fmt, io};

use mio::{event, Interest, Token};

use crate::actor::{self, AddActorError, NewActor, PrivateSpawn, Spawn};
use crate::actor_ref::ActorRef;
use crate::rt::process::ProcessId;
use crate::rt::{ActorOptions, RuntimeRef};
use crate::supervisor::Supervisor;

/// Trait to indicate an API needs access to the Heph runtime.
///
/// This is used by various API to get access to the runtime, but its only
/// usable inside the Heph crate.
///
/// # Notes
///
/// This trait can't be implemented by types outside of the Heph crate.
pub trait Access: PrivateAccess {}

/// Actual trait behind [`rt::Access`].
///
/// [`rt::Access`]: crate::rt::Access
pub trait PrivateAccess {
    /// Registers the `source` using `token` and `interest`.
    fn register<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized;

    /// Reregisters the `source` using `token` and `interest`.
    fn reregister<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized;

    /// Add a deadline for `pid` at `deadline`.
    fn add_deadline(&mut self, pid: ProcessId, deadline: Instant);

    /// Returns the CPU the thread is bound to, if any.
    fn cpu(&self) -> Option<usize>;
}

/// Provides access to the thread-local runtime.
///
/// This implements the [`Access`] trait, which is required by various APIs to
/// get access to the runtime. Furthermore this gives access to a
/// [`RuntimeRef`] for users.
///
/// This is usually a part of the [`actor::Context`], see it for more
/// information.
///
/// This is an optimised version of [`ThreadSafe`], but doesn't allow the actor
/// to move between threads.
///
/// [`actor::Context`]: crate::actor::Context
pub struct ThreadLocal {
    rt: RuntimeRef,
}

impl ThreadLocal {
    pub(crate) const fn new(rt: RuntimeRef) -> ThreadLocal {
        ThreadLocal { rt }
    }
}

impl Deref for ThreadLocal {
    type Target = RuntimeRef;

    fn deref(&self) -> &Self::Target {
        &self.rt
    }
}

impl DerefMut for ThreadLocal {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.rt
    }
}

impl Access for ThreadLocal {}

impl PrivateAccess for ThreadLocal {
    fn register<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.rt.register(source, token, interest)
    }

    fn reregister<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.rt.reregister(source, token, interest)
    }

    fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.rt.add_deadline(pid, deadline)
    }

    fn cpu(&self) -> Option<usize> {
        self.rt.cpu()
    }
}

impl<S, NA> Spawn<S, NA, ThreadLocal> for ThreadLocal {}

impl<S, NA> PrivateSpawn<S, NA, ThreadLocal> for ThreadLocal {
    fn try_spawn_setup<ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<RuntimeAccess = ThreadLocal> + 'static,
        NA::Actor: 'static,
        ArgFn:
            FnOnce(&mut actor::Context<NA::Message, ThreadLocal>) -> Result<NA::Argument, ArgFnE>,
    {
        self.rt
            .try_spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}

impl fmt::Debug for ThreadLocal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThreadLocal")
    }
}
