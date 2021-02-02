//! Synchronous actor thread code.

use std::io::{self, Write};
use std::thread;

use inbox::ReceiverConnected;
use log::{trace, warn};
use mio::unix;
use mio::{Interest, Registry, Token};

use crate::actor::sync::{SyncActor, SyncContext};
use crate::rt::options::SyncActorOptions;
use crate::supervisor::{SupervisorStrategy, SyncSupervisor};
use crate::trace;
use crate::ActorRef;

/// Handle to a synchronous worker.
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
    pub(crate) fn start<Sv, A, E, Arg, M>(
        id: usize,
        supervisor: Sv,
        actor: A,
        arg: Arg,
        options: SyncActorOptions,
        trace_log: Option<trace::Log>,
    ) -> io::Result<(SyncWorker, ActorRef<M>)>
    where
        Sv: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
        Arg: Send + 'static,
        M: Send + 'static,
    {
        unix::pipe::new().and_then(|(sender, receiver)| {
            let (manager, send, _) = inbox::Manager::new_small_channel();
            let actor_ref = ActorRef::local(send);
            let thread_name = options
                .thread_name
                .unwrap_or_else(|| format!("Sync actor {}", id));
            thread::Builder::new()
                .name(thread_name)
                .spawn(move || main(supervisor, actor, arg, manager, receiver, trace_log))
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
    pub(super) fn is_alive(&mut self) -> bool {
        match self.sender.write(&[]) {
            Ok(..) => true,
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => true,
            Err(..) => false,
        }
    }

    /// See [`thread::JoinHandle::join`].
    pub(super) fn join(self) -> thread::Result<()> {
        self.handle.join()
    }

    /// Returns the [`thread::Handle`].
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn into_handle(self) -> thread::JoinHandle<()> {
        self.handle
    }
}

/// Run a synchronous actor worker thread.
fn main<S, E, Arg, A, M>(
    mut supervisor: S,
    actor: A,
    mut arg: Arg,
    inbox: inbox::Manager<M>,
    receiver: unix::pipe::Receiver,
    mut trace_log: Option<trace::Log>,
) where
    S: SyncSupervisor<A> + 'static,
    A: SyncActor<Message = M, Argument = Arg, Error = E>,
{
    trace!("running synchronous actor");
    loop {
        let timing = trace::start(&trace_log);
        let receiver = inbox.new_receiver().unwrap_or_else(inbox_failure);
        let ctx = SyncContext::new(receiver, clone_trace_log(&mut trace_log));
        trace::finish(&mut trace_log, timing, "setting up synchronous actor", &[]);

        let timing = trace::start(&trace_log);
        let res = actor.run(ctx, arg);
        trace::finish(&mut trace_log, timing, "running synchronous actor", &[]);

        match res {
            Ok(()) => break,
            Err(err) => {
                let timing = trace::start(&trace_log);
                match supervisor.decide(err) {
                    SupervisorStrategy::Restart(new_arg) => {
                        trace!("restarting synchronous actor");
                        arg = new_arg;
                        trace::finish(&mut trace_log, timing, "restarting synchronous actor", &[]);
                    }
                    SupervisorStrategy::Stop => {
                        trace::finish(&mut trace_log, timing, "stopping synchronous actor", &[]);
                        break;
                    }
                }
            }
        }
    }

    trace!("stopping synchronous actor");
    // First drop all values as this might take an arbiterary time.
    drop(actor);
    drop(supervisor);
    drop(inbox);
    drop(trace_log);
    // After dropping all values let the coordinator know we're done.
    drop(receiver);
}

#[cold]
fn inbox_failure<T>(_: ReceiverConnected) -> T {
    panic!("failed to create new receiver for synchronous actor's inbox. Was the `SyncContext` leaked?");
}

fn clone_trace_log(trace_log: &mut Option<trace::Log>) -> Option<trace::Log> {
    if let Some(t_log) = trace_log.as_ref() {
        match t_log.try_clone() {
            Ok(trace_log) => Some(trace_log),
            Err(err) => {
                warn!("failed to clone trace log: {}, disabling tracing for synchronous actor on next restart", err);
                trace_log.take()
            }
        }
    } else {
        None
    }
}
