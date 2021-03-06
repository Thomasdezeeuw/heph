//! Synchronous actor thread code.

use std::io::{self, Write};
use std::thread;

use heph_inbox::{self as inbox, ReceiverConnected};
use log::trace;
use mio::{unix, Interest, Registry, Token};

use crate::actor::{SyncActor, SyncContext};
use crate::actor_ref::ActorRef;
use crate::spawn::options::SyncActorOptions;
use crate::supervisor::{SupervisorStrategy, SyncSupervisor};
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
        options: SyncActorOptions,
        trace_log: Option<trace::Log>,
    ) -> io::Result<(SyncWorker, ActorRef<A::Message>)>
    where
        S: SyncSupervisor<A> + Send + 'static,
        A: SyncActor + Send + 'static,
        A::Message: Send + 'static,
        A::Argument: Send + 'static,
    {
        unix::pipe::new().and_then(|(sender, receiver)| {
            let (manager, send, _) = inbox::Manager::new_small_channel();
            let actor_ref = ActorRef::local(send);
            let thread_name = options
                .take_name()
                .unwrap_or_else(|| format!("Sync actor {}", id));
            thread::Builder::new()
                .name(thread_name)
                .spawn(move || main(id, supervisor, actor, arg, manager, receiver, trace_log))
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

    /// Returns the [`thread::JoinHandle`].
    #[cfg(any(test, feature = "test"))]
    pub(crate) fn into_handle(self) -> thread::JoinHandle<()> {
        self.handle
    }
}

/// Run a synchronous actor worker thread.
fn main<S, A>(
    id: usize,
    mut supervisor: S,
    actor: A,
    mut arg: A::Argument,
    inbox: inbox::Manager<A::Message>,
    receiver: unix::pipe::Receiver,
    mut trace_log: Option<trace::Log>,
) where
    S: SyncSupervisor<A> + 'static,
    A: SyncActor,
{
    let thread = thread::current();
    let name = thread.name().unwrap();
    trace!("running synchronous actor: pid={}, name='{}'", id, name);
    loop {
        let timing = trace::start(&trace_log);
        let receiver = inbox.new_receiver().unwrap_or_else(inbox_failure);
        let ctx = SyncContext::new(receiver, trace_log.clone());
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
                        trace!("restarting synchronous actor: pid={}, name='{}'", id, name);
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
                }
            }
        }
    }

    trace!("stopping synchronous actor: pid={}, name='{}'", id, name);
    // First drop all values as this might take an arbitrary time.
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
