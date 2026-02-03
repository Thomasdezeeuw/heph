//! Synchronous actor thread code.
//!
//! A sync worker manages and runs a single synchronous actor. It simply runs
//! the sync actor and handles any errors it returns (by restarting the actor or
//! stopping the thread).
//!
//! The [`sync_worker::Handle`] type is a handle to the sync worker thread
//! managed by the [coordinator]. The [`start`] function can be used to start a
//! new synchronous actor.
//!
//! [coordinator]: crate::coordinator
//! [`sync_worker::Handle`]: Handle

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::{io, thread};

use heph::actor_ref::ActorRef;
use heph::supervisor::SyncSupervisor;
use heph::sync::{SyncActor, SyncActorRunnerBuilder};

use crate::spawn::options::SyncActorOptions;
use crate::trace;
use crate::{self as rt, shared};

/// Spawn a new thread to run the synchronous actor.
pub(crate) fn spawn_thread<S, A>(
    id: NonZeroUsize,
    supervisor: S,
    actor: A,
    arg: A::Argument,
    options: SyncActorOptions,
    shared: Arc<shared::RuntimeInternals>,
    trace_log: Option<trace::Log>,
) -> io::Result<(Handle, ActorRef<A::Message>)>
where
    S: SyncSupervisor<A> + Send + 'static,
    A: SyncActor<RuntimeAccess = rt::Sync> + Send + 'static,
    A::Message: Send + 'static,
    A::Argument: Send + 'static,
{
    let (runner, actor_ref) = SyncActorRunnerBuilder::new()
        .with_rt(rt::Sync::new(shared.clone(), trace_log))
        .with_inbox_size(options.inbox_size())
        .build(supervisor, actor);
    let thread_name = options
        .take_thread_name()
        .unwrap_or_else(|| A::name().to_owned());
    let handle = thread::Builder::new().name(thread_name).spawn(move || {
        let _wake_coordinator_on_drop = WakeOnDrop(shared); // Only needs to be dropped.
        runner.run(arg);
    })?;
    Ok((Handle { id, handle }, actor_ref))
}

/// Calls [`shared::RuntimeInternals::wake_coordinator`] when the type is
/// dropped.
struct WakeOnDrop(Arc<shared::RuntimeInternals>);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        // Wake the coordinator forcing it check if the sync workers are still alive.
        self.0.wake_coordinator();
    }
}

/// Handle to a synchronous worker.
#[derive(Debug)]
pub(crate) struct Handle {
    /// Unique id among all threads in the `Runtime`.
    id: NonZeroUsize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<()>,
}

impl Handle {
    /// Return the worker's id.
    pub(crate) const fn id(&self) -> NonZeroUsize {
        self.id
    }

    /// See [`thread::JoinHandle::is_finished`].
    pub(crate) fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// See [`thread::JoinHandle::join`].
    pub(crate) fn join(self) -> Result<(), rt::Error> {
        self.handle.join().map_err(rt::Error::sync_actor_panic)
    }

    /// Returns the [`thread::JoinHandle`].
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn into_handle(self) -> thread::JoinHandle<()> {
        self.handle
    }
}
