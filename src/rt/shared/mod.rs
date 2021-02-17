//! Module with shared runtime internals.

use std::cmp::min;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::{io, task};

use log::{debug, error, trace};
use mio::{event, Interest, Registry, Token};

use crate::actor::context::ThreadSafe;
use crate::actor::{self, AddActorError, NewActor};
use crate::actor_ref::ActorRef;
use crate::rt::thread_waker::ThreadWaker;
use crate::rt::timers::Timers;
use crate::rt::{ActorOptions, ProcessId};
use crate::supervisor::Supervisor;

mod scheduler;
pub(crate) mod waker;

use waker::WakerId;

pub(crate) use scheduler::{ProcessData, Scheduler};

/// Shared internals of the runtime.
#[derive(Debug)]
pub(crate) struct RuntimeInternals {
    /// Waker id used to create a `Waker` for thread-safe actors.
    coordinator_id: WakerId,
    /// Thread waker for the coordinator.
    coordinator_waker: ThreadWaker,
    /// Thread wakers for all the workers.
    worker_wakers: Box<[&'static ThreadWaker]>,
    /// Index into `worker_wakers` to wake next, see
    /// [`RuntimeInternals::wake_workers`].
    wake_worker_idx: AtomicUsize,
    /// Scheduler for thread-safe actors.
    scheduler: Scheduler,
    /// Registry for the `Coordinator`'s `Poll` instance.
    registry: Registry,
    /// Timers for thread-safe actors.
    // FIXME: `Timers` is not up to this job.
    timers: Mutex<Timers>,
}

impl RuntimeInternals {
    pub(crate) fn new(
        coordinator_id: WakerId,
        coordinator_waker: mio::Waker,
        worker_wakers: Box<[&'static ThreadWaker]>,
        scheduler: Scheduler,
        registry: Registry,
        timers: Mutex<Timers>,
    ) -> RuntimeInternals {
        // Needed by `RuntimeInternals::wake_workers`.
        debug_assert!(worker_wakers.len() >= 1);
        RuntimeInternals {
            coordinator_id,
            coordinator_waker: ThreadWaker::new(coordinator_waker),
            worker_wakers,
            wake_worker_idx: AtomicUsize::new(0),
            scheduler,
            registry,
            timers,
        }
    }

    /// Returns a new [`task::Waker`] for the thread-safe actor with `pid`.
    pub(crate) fn new_task_waker(&self, pid: ProcessId) -> task::Waker {
        waker::new(self.coordinator_id, pid)
    }

    /// Register an `event::Source`, see [`mio::Registry::register`].
    pub(crate) fn register<S>(
        &self,
        source: &mut S,
        token: Token,
        interest: Interest,
    ) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.registry.register(source, token, interest)
    }

    /// Reregister an `event::Source`, see [`mio::Registry::reregister`].
    pub(crate) fn reregister<S>(
        &self,
        source: &mut S,
        token: Token,
        interest: Interest,
    ) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.registry.reregister(source, token, interest)
    }

    pub(crate) fn add_deadline(&self, pid: ProcessId, deadline: Instant) {
        self.timers.lock().unwrap().add_deadline(pid, deadline);
        // Ensure that the coordinator isn't polling and misses the deadline.
        self.wake_coordinator()
    }

    /// Waker used to wake the `Coordinator`, but not schedule any particular
    /// process.
    fn wake_coordinator(&self) {
        if let Err(err) = self.coordinator_waker.wake() {
            error!("unable to wake up coordinator: {}", err);
        }
    }

    #[allow(clippy::needless_pass_by_value)] // For `ActorOptions`.
    pub(crate) fn spawn_setup<S, NA, ArgFn, ArgFnE>(
        self: &Arc<Self>,
        supervisor: S,
        mut new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Context = ThreadSafe> + Sync + Send + 'static,
        ArgFn: FnOnce(&mut actor::Context<NA::Message, ThreadSafe>) -> Result<NA::Argument, ArgFnE>,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        // Setup adding a new process to the scheduler.
        let actor_entry = self.scheduler.add_actor();
        let pid = actor_entry.pid();
        let name = actor::name::<NA::Actor>();
        debug!("spawning thread-safe actor: pid={}, name={}", pid, name);

        // Create our actor context and our actor with it.
        let (manager, sender, receiver) = inbox::Manager::new_small_channel();
        let actor_ref = ActorRef::local(sender);
        let mut ctx = actor::Context::new_shared(pid, receiver, self.clone());
        let arg = arg_fn(&mut ctx).map_err(AddActorError::ArgFn)?;
        let actor = new_actor.new(ctx, arg).map_err(AddActorError::NewActor)?;

        // Add the actor to the scheduler.
        actor_entry.add(
            options.priority(),
            supervisor,
            new_actor,
            actor,
            manager,
            options.is_ready(),
        );

        Ok(actor_ref)
    }

    /// See [`Scheduler::mark_ready`].
    pub(crate) fn mark_ready(&self, pid: ProcessId) {
        self.scheduler.mark_ready(pid)
    }

    /// Wake `n` worker threads.
    pub(crate) fn wake_workers(&self, n: usize) {
        trace!("waking {} worker thread(s)", n);
        // To prevent the Thundering herd problem [1] we don't wake all workers,
        // only enough worker threads to handle all events. To spread the
        // workload (somewhat more) evenly we wake the workers in a Round-Robin
        // [2] fashion.
        //
        // [1]: https://en.wikipedia.org/wiki/Thundering_herd_problem
        // [2]: https://en.wikipedia.org/wiki/Round-robin_scheduling
        let n = min(n, self.worker_wakers.len());
        let wake_worker_idx =
            self.wake_worker_idx.fetch_add(n, Ordering::AcqRel) % self.worker_wakers.len();
        let (wake_second, wake_first) = self.worker_wakers.split_at(wake_worker_idx);
        let workers_to_wake = wake_first.iter().chain(wake_second.iter());
        let mut wakes_left = n;
        for worker in workers_to_wake {
            match worker.wake() {
                Ok(true) => {
                    wakes_left -= 1;
                    if wakes_left == 0 {
                        break;
                    }
                }
                Ok(false) => {}
                Err(err) => error!("error waking worker: {}", err),
            }
        }
    }

    // Worker only API.

    /// See [`Scheduler::has_process`].
    pub(crate) fn has_process(&self) -> bool {
        self.scheduler.has_process()
    }

    /// See [`Scheduler::has_ready_process`].
    pub(crate) fn has_ready_process(&self) -> bool {
        self.scheduler.has_ready_process()
    }

    /// See [`Scheduler::remove`].
    pub(crate) fn remove_process(&self) -> Option<Pin<Box<ProcessData>>> {
        self.scheduler.remove()
    }

    /// See [`Scheduler::add_process`].
    pub(crate) fn add_process(&self, process: Pin<Box<ProcessData>>) {
        self.scheduler.add_process(process);
    }

    /// See [`Scheduler::complete`].
    pub(crate) fn complete(&self, process: Pin<Box<ProcessData>>) {
        self.scheduler.complete(process);
    }

    // Coordinator only API.

    pub(crate) fn timers(&self) -> &Mutex<Timers> {
        &self.timers
    }

    pub(crate) fn mark_polling(&self, polling: bool) {
        self.coordinator_waker.mark_polling(polling)
    }
}
