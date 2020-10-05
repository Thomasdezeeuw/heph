//! Synchronous actor thread code.

use std::ptr::NonNull;
use std::{io, thread};

use crossbeam_channel::{self as channel, Receiver};
use log::trace;
use mio::{event, Interest, Registry, Token};
use mio_pipe::new_pipe;

use crate::actor::sync::{SyncActor, SyncContext, SyncContextData};
use crate::rt::options::SyncActorOptions;
use crate::supervisor::{SupervisorStrategy, SyncSupervisor};
use crate::ActorRef;

/// Handle to a synchronous worker.
pub(super) struct SyncWorker {
    /// Unique id (among all threads in the `Runtime`).
    id: usize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<()>,
    /// Sending half of the Unix pipe, used to communicate with the thread.
    sender: mio_pipe::Sender,
}

impl SyncWorker {
    /// Start a new thread that runs a synchronous actor.
    pub(super) fn start<Sv, A, E, Arg, M>(
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
        new_pipe().and_then(|(sender, receiver)| {
            let (send, inbox) = channel::unbounded();
            let actor_ref = ActorRef::for_sync_actor(send);
            let thread_name = options
                .thread_name
                .unwrap_or_else(|| format!("Sync actor {}", id));
            thread::Builder::new()
                .name(thread_name)
                .spawn(move || main(supervisor, actor, arg, inbox, receiver))
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
    inbox: Receiver<M>,
    receiver: mio_pipe::Receiver,
) where
    S: SyncSupervisor<A> + 'static,
    A: SyncActor<Message = M, Argument = Arg, Error = E>,
{
    trace!("running synchronous actor");
    let mut ctx_data = SyncContextData::new(inbox);

    loop {
        // This is safe because the context data doesn't outlive the pointer
        // and the pointer is not null.
        let ctx = unsafe { SyncContext::new(NonNull::new_unchecked(&mut ctx_data)) };

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
