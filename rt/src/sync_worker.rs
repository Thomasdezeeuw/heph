//! Synchronous actor thread code.
//!
//! A sync worker manages and runs a single synchronous actor. It simply runs
//! the sync actor and handles any errors it returns (by restarting the actor or
//! stopping the thread).
//!
//! The [`sync_worker::Handle`] type is a handle to the sync worker thread
//! managed by the [coordinator]. The [`main`] function is the entry point for
//! the sync worker thread.
//!
//! [coordinator]: crate::coordinator
//! [`sync_worker::Handle`]: Handle

use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::{io, thread};

use heph::actor_ref::ActorRef;
use heph::supervisor::{SupervisorStrategy, SyncSupervisor};
use heph::sync::{SyncActor, SyncContext};
use heph_inbox::{self as inbox, ReceiverConnected};
use log::trace;

use crate::spawn::options::SyncActorOptions;
use crate::trace;
use crate::{self as rt, shared};

/// Start a new thread that runs a synchronous actor.
pub(crate) fn start<S, A>(
    id: usize,
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
    let (inbox, sender, ..) = inbox::Manager::new_small_channel();
    let actor_ref = ActorRef::local(sender);
    let thread_name = options
        .take_name()
        .unwrap_or_else(|| format!("Sync actor {id}"));
    let handle = thread::Builder::new().name(thread_name).spawn(move || {
        #[rustfmt::skip]
        let worker = SyncWorker { id, supervisor, actor, inbox, shared, trace_log };
        worker.run(arg);
    })?;
    Ok((Handle { id, handle }, actor_ref))
}

/// Handle to a synchronous worker.
#[derive(Debug)]
pub(crate) struct Handle {
    /// Unique id among all threads in the `Runtime`.
    id: usize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<()>,
}

impl Handle {
    /// Return the worker's id.
    pub(crate) const fn id(&self) -> usize {
        self.id
    }

    /// See [`thread::JoinHandle::is_finished`].
    pub(crate) fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// See [`thread::JoinHandle::join`].
    pub(crate) fn join(self) -> thread::Result<()> {
        self.handle.join()
    }

    /// Returns the [`thread::JoinHandle`].
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn into_handle(self) -> thread::JoinHandle<()> {
        self.handle
    }
}

/// Worker that runs a single synchronous actor.
struct SyncWorker<S: SyncSupervisor<A>, A: SyncActor<RuntimeAccess = rt::Sync>> {
    id: usize,
    supervisor: S,
    actor: A,
    inbox: inbox::Manager<A::Message>,
    shared: Arc<shared::RuntimeInternals>,
    trace_log: Option<trace::Log>,
}

impl<S: SyncSupervisor<A>, A: SyncActor<RuntimeAccess = rt::Sync>> SyncWorker<S, A> {
    /// Run the synchronous actor.
    fn run(mut self, mut arg: A::Argument) {
        let thread = thread::current();
        let name = thread.name().unwrap();
        trace!(sync_worker_id = self.id, name = name; "running synchronous actor");
        loop {
            let timing = trace::start(&self.trace_log);
            let receiver = self.inbox.new_receiver().unwrap_or_else(inbox_failure);
            let rt = rt::Sync::new(self.shared.clone(), self.trace_log.clone());
            let ctx = SyncContext::new(receiver, rt);
            trace::finish_rt(
                self.trace_log.as_mut(),
                timing,
                "setting up synchronous actor",
                &[],
            );

            let timing = trace::start(&self.trace_log);
            let res = panic::catch_unwind(AssertUnwindSafe(|| self.actor.run(ctx, arg)));
            trace::finish_rt(
                self.trace_log.as_mut(),
                timing,
                "running synchronous actor",
                &[],
            );

            match res {
                Ok(Ok(())) => break,
                Ok(Err(err)) => {
                    let timing = trace::start(&self.trace_log);
                    match self.supervisor.decide(err) {
                        SupervisorStrategy::Restart(new_arg) => {
                            trace!(sync_worker_id = self.id, name = name; "restarting synchronous actor");
                            arg = new_arg;
                            trace::finish_rt(
                                self.trace_log.as_mut(),
                                timing,
                                "restarting synchronous actor",
                                &[],
                            );
                        }
                        SupervisorStrategy::Stop => {
                            trace::finish_rt(
                                self.trace_log.as_mut(),
                                timing,
                                "stopping synchronous actor",
                                &[],
                            );
                            break;
                        }
                        _ => unreachable!(),
                    }
                }
                Err(panic) => {
                    let timing = trace::start(&self.trace_log);
                    match self.supervisor.decide_on_panic(panic) {
                        SupervisorStrategy::Restart(new_arg) => {
                            trace!(sync_worker_id = self.id, name = name; "restarting synchronous actor after panic");
                            arg = new_arg;
                            trace::finish_rt(
                                self.trace_log.as_mut(),
                                timing,
                                "restarting synchronous actor after panic",
                                &[],
                            );
                        }
                        SupervisorStrategy::Stop => {
                            trace::finish_rt(
                                self.trace_log.as_mut(),
                                timing,
                                "stopping synchronous actor after panic",
                                &[],
                            );
                            break;
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        trace!(sync_worker_id = self.id, name = name; "stopping synchronous actor");
    }
}

impl<S: SyncSupervisor<A>, A: SyncActor<RuntimeAccess = rt::Sync>> Drop for SyncWorker<S, A> {
    fn drop(&mut self) {
        // Wake the coordinator forcing it check if the sync workers are still alive.
        self.shared.wake_coordinator();
    }
}

/// Called when we can't create a new receiver for the sync actor.
#[cold]
fn inbox_failure<T>(_: ReceiverConnected) -> T {
    panic!("failed to create new receiver for synchronous actor's inbox. Was the `SyncContext` leaked?");
}
