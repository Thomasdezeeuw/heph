//! Synchronous actor thread code.

use std::{io, thread};

use inbox::Manager;
use log::trace;
use mio::unix::pipe;
use mio::{event, Interest, Registry, Token};

use crate::actor::sync::{SyncActor, SyncContext};
use crate::rt::options::SyncActorOptions;
use crate::supervisor::{SupervisorStrategy, SyncSupervisor};
use crate::ActorRef;

/// Handle to a synchronous worker.
pub(crate) struct SyncWorker {
    /// Unique id (among all threads in the `Runtime`).
    id: usize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<()>,
    /// Sending half of the Unix pipe, used to communicate with the thread.
    sender: pipe::Sender,
}

impl SyncWorker {
    /// Start a new thread that runs a synchronous actor.
    pub(crate) fn start<Sv, A, E, Arg, M>(
        id: usize,
        supervisor: Sv,
        actor: A,
        arg: Arg,
        options: SyncActorOptions,
    ) -> io::Result<(SyncWorker, ActorRef<M>)>
    where
        Sv: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
        Arg: Send + 'static,
        M: Send + 'static,
    {
        pipe::new().and_then(|(sender, receiver)| {
            let (manager, send, ..) = inbox::Manager::new_small_channel();
            let actor_ref = ActorRef::local(send);
            let thread_name = options
                .thread_name
                .unwrap_or_else(|| format!("Sync actor {}", id));
            thread::Builder::new()
                .name(thread_name)
                .spawn(move || main(supervisor, actor, arg, manager, receiver))
                .map(|handle| SyncWorker { id, handle, sender })
                .map(|worker| (worker, actor_ref))
        })
    }

    /// Return the worker's id.
    pub(super) const fn id(&self) -> usize {
        self.id
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

/// Registers the sending end of the Unix pipe used to communicate with the
/// thread.
impl event::Source for SyncWorker {
    fn register(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        self.sender.register(registry, token, interests)
    }

    fn reregister(
        &mut self,
        registry: &Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        self.sender.reregister(registry, token, interests)
    }

    fn deregister(&mut self, registry: &Registry) -> io::Result<()> {
        self.sender.deregister(registry)
    }
}

/// Run a synchronous actor worker thread.
fn main<S, E, Arg, A, M>(
    mut supervisor: S,
    actor: A,
    mut arg: Arg,
    inbox: Manager<M>,
    receiver: pipe::Receiver,
) where
    S: SyncSupervisor<A> + 'static,
    A: SyncActor<Message = M, Argument = Arg, Error = E>,
{
    trace!("running synchronous actor");
    loop {
        let receiver = inbox.new_receiver().expect(
            "failed to create new receiver for actor's inbox. Was the `SyncContext` leaked?",
        );
        let ctx = SyncContext::new(receiver);

        match actor.run(ctx, arg) {
            Ok(()) => break,
            Err(err) => match supervisor.decide(err) {
                SupervisorStrategy::Restart(new_arg) => {
                    trace!("restarting synchronous actor");
                    arg = new_arg
                }
                SupervisorStrategy::Stop => break,
            },
        }
    }

    trace!("stopping synchronous actor");
    // Let the coordinator know we're done.
    drop(receiver);
}
