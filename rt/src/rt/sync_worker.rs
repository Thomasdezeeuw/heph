//! Synchronous actor thread code.
//!
//! A sync worker manages and runs a single synchronous actor. It simply runs
//! the sync actor and handles any errors it returns (by restarting the actor or
//! stopping the thread).
//!
//! The [`SyncWorker`] type is a handle to the sync worker thread managed by the
//! [coordinator]. The [`main`] function is the entry point for the sync worker
//! thread.
//!
//! [coordinator]: crate::rt::coordinator

use std::io::{self, Write};
use std::sync::Arc;
use std::thread;

use heph::actor::{SyncActor, SyncContext};
use heph::actor_ref::ActorRef;
use heph::spawn::options::SyncActorOptions;
use heph::supervisor::{SupervisorStrategy, SyncSupervisor};
use heph_inbox::{self as inbox, ReceiverConnected};
use log::trace;
use mio::{unix, Interest, Registry, Token};

use crate::rt::{self, shared};
use crate::trace;

/// Handle to a synchronous worker.
#[derive(Debug)]
pub(crate) struct SyncWorker {
    /// Unique id among all threads in the `Runtime`.
    id: usize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<()>,
    /// Sending half of the Unix pipe, used to communicate with the thread.
    sender: unix::pipe::Sender,
}

impl SyncWorker {
    /// Start a new thread that runs a synchronous actor.
    pub(crate) fn start<S, A>(
        id: usize,
        supervisor: S,
        actor: A,
        arg: A::Argument,
        options: SyncActorOptions<()>,
        rt: Arc<shared::RuntimeInternals>,
        trace_log: Option<trace::Log>,
    ) -> io::Result<(SyncWorker, ActorRef<A::Message>)>
    where
        S: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<RuntimeAccess = rt::Sync> + Send + 'static,
        A::Message: Send + 'static,
        A::Argument: Send + 'static,
    {
        unix::pipe::new().and_then(|(sender, receiver)| {
            let (manager, send, ..) = inbox::Manager::new_small_channel();
            let actor_ref = ActorRef::local(send);
            let thread_name = options
                .take_name()
                .unwrap_or_else(|| format!("Sync actor {}", id));
            thread::Builder::new()
                .name(thread_name)
                .spawn(move || main(id, supervisor, actor, arg, manager, receiver, rt, trace_log))
                .map(|handle| (SyncWorker { id, handle, sender }, actor_ref))
        })
    }

    /// Return the worker's id.
    pub(super) const fn id(&self) -> usize {
        self.id
    }

    /// Registers the sending end of the Unix pipe used to communicate with the
    /// thread. Uses the [`id`] as [`Token`].
    ///
    /// [`id`]: SyncWorker::id
    pub(super) fn register(&mut self, registry: &Registry) -> io::Result<()> {
        registry.register(&mut self.sender, Token(self.id), Interest::WRITABLE)
    }

    /// Checks if the `SyncWorker` is alive.
    pub(super) fn is_alive(&self) -> bool {
        match (&self.sender).write(&[]) {
            Ok(..) => true,
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => true,
            Err(..) => false,
        }
    }

    /// See [`thread::JoinHandle::join`].
    pub(super) fn join(self) -> thread::Result<()> {
        self.handle.join()
    }

    /// Returns the [`thread::JoinHandle`].
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn into_handle(self) -> thread::JoinHandle<()> {
        self.handle
    }
}

/// Run a synchronous actor worker thread.
#[allow(clippy::too_many_arguments)]
fn main<S, A>(
    id: usize,
    mut supervisor: S,
    actor: A,
    mut arg: A::Argument,
    inbox: inbox::Manager<A::Message>,
    receiver: unix::pipe::Receiver,
    rt: Arc<shared::RuntimeInternals>,
    mut trace_log: Option<trace::Log>,
) where
    S: SyncSupervisor<A> + 'static,
    A: SyncActor<RuntimeAccess = rt::Sync>,
{
    let thread = thread::current();
    let name = thread.name().unwrap();
    trace!(sync_worker_id = id, name = name; "running synchronous actor");
    loop {
        let timing = trace::start(&trace_log);
        let receiver = inbox.new_receiver().unwrap_or_else(inbox_failure);
        let rt = rt::Sync::new(rt.clone(), trace_log.clone());
        let ctx = SyncContext::new(receiver, rt);
        trace::finish_rt(
            trace_log.as_mut(),
            timing,
            "setting up synchronous actor",
            &[],
        );

        let timing = trace::start(&trace_log);
        let res = actor.run(ctx, arg);
        trace::finish_rt(trace_log.as_mut(), timing, "running synchronous actor", &[]);

        match res {
            Ok(()) => break,
            Err(err) => {
                let timing = trace::start(&trace_log);
                match supervisor.decide(err) {
                    SupervisorStrategy::Restart(new_arg) => {
                        trace!(sync_worker_id = id, name = name; "restarting synchronous actor");
                        arg = new_arg;
                        trace::finish_rt(
                            trace_log.as_mut(),
                            timing,
                            "restarting synchronous actor",
                            &[],
                        );
                    }
                    SupervisorStrategy::Stop => {
                        trace::finish_rt(
                            trace_log.as_mut(),
                            timing,
                            "stopping synchronous actor",
                            &[],
                        );
                        break;
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    trace!(sync_worker_id = id, name = name; "stopping synchronous actor");
    // First drop all values as this might take an arbitrary time.
    drop(actor);
    drop(supervisor);
    drop(inbox);
    drop(rt);
    drop(trace_log);
    // After dropping all values let the coordinator know we're done.
    drop(receiver);
}

/// Called when we can't create a new receiver for the sync actor.
#[cold]
fn inbox_failure<T>(_: ReceiverConnected) -> T {
    panic!("failed to create new receiver for synchronous actor's inbox. Was the `SyncContext` leaked?");
}
