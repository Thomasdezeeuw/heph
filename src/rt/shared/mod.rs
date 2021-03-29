//! Module with shared runtime internals.

use std::cmp::min;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{io, task};

use log::{debug, error, trace};
use mio::{event, Interest, Registry, Token};

use crate::actor::{self, NewActor};
use crate::actor_ref::ActorRef;
use crate::rt::thread_waker::ThreadWaker;
use crate::rt::{ProcessId, ThreadSafe};
use crate::spawn::{ActorOptions, AddActorError, FutureOptions};
use crate::supervisor::Supervisor;
use crate::trace;

mod scheduler;
mod timers;
pub(crate) mod waker;

pub(crate) use timers::Timers;
use waker::WakerId;

pub(crate) use scheduler::{ProcessData, Scheduler};

/// Shared internals of the runtime.
#[derive(Debug)]
pub(crate) struct RuntimeInternals {
    /// Waker id used to create [`task::Waker`]s for thread-safe actors.
    shared_id: WakerId,
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
    timers: Timers,
    /// Shared trace log.
    ///
    /// # Notes
    ///
    /// Prefer not to use this but use [`trace::Log`] in local internals
    /// instead.
    trace_log: Option<Arc<trace::SharedLog>>,
}

impl RuntimeInternals {
    pub(crate) fn new(
        shared_id: WakerId,
        worker_wakers: Box<[&'static ThreadWaker]>,
        scheduler: Scheduler,
        registry: Registry,
        timers: Timers,
        trace_log: Option<Arc<trace::SharedLog>>,
    ) -> RuntimeInternals {
        // Needed by `RuntimeInternals::wake_workers`.
        debug_assert!(worker_wakers.len() >= 1);
        RuntimeInternals {
            shared_id,
            worker_wakers,
            wake_worker_idx: AtomicUsize::new(0),
            scheduler,
            registry,
            timers,
            trace_log,
        }
    }

    /// Returns a new [`task::Waker`] for the thread-safe actor with `pid`.
    pub(crate) fn new_task_waker(&self, pid: ProcessId) -> task::Waker {
        waker::new(self.shared_id, pid)
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

    /// See [`Timers::add`].
    pub(super) fn add_deadline(&self, pid: ProcessId, deadline: Instant) {
        self.timers.add(pid, deadline);
    }

    /// See [`Timers::remove`].
    pub(super) fn remove_deadline(&self, pid: ProcessId, deadline: Instant) {
        self.timers.remove(pid, deadline);
    }

    /// See [`Timers::change`].
    pub(super) fn change_deadline(&self, from: ProcessId, to: ProcessId, deadline: Instant) {
        self.timers.change(from, deadline, to);
    }

    /// See [`Timers::remove_deadline`].
    pub(crate) fn remove_next_deadline(&self, now: Instant) -> Option<ProcessId> {
        self.timers.remove_next(now)
    }

    /// Determine the timeout to use in polling based on the current time
    /// (`now`), the `current` timeout and the next deadline in the shared
    /// timers.
    ///
    /// If there are no timers this will return `current`. If `current` is
    /// smaller than the next deadline in the timers this will also
    /// return `current`. Otherwise this will return a timeout based on the
    /// next deadline.
    pub(crate) fn next_timeout(
        &self,
        now: Instant,
        current: Option<Duration>,
    ) -> (Option<Duration>, bool) {
        match self.timers.next() {
            Some(deadline) => match deadline.checked_duration_since(now) {
                // Timer has already expired, so no blocking.
                None => (Some(Duration::ZERO), true),
                Some(timeout) => match current {
                    Some(current) if current < timeout => {
                        // Not using the shared timers timeout.
                        self.timers.woke_from_polling();
                        (Some(current), false)
                    }
                    Some(..) | None => (Some(timeout), true),
                },
            },
            None => (current, false),
        }
    }

    pub(super) fn timers_woke_from_polling(&self) {
        self.timers.woke_from_polling()
    }

    #[allow(clippy::needless_pass_by_value)] // For `ActorOptions`.
    pub(crate) fn spawn_setup<S, NA, ArgFn, E>(
        self: &Arc<Self>,
        supervisor: S,
        mut new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, E>>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<RuntimeAccess = ThreadSafe> + Sync + Send + 'static,
        ArgFn: FnOnce(&mut actor::Context<NA::Message, ThreadSafe>) -> Result<NA::Argument, E>,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        // Setup adding a new process to the scheduler.
        let actor_entry = self.scheduler.add_actor();
        let pid = actor_entry.pid();
        let name = new_actor.name();
        debug!("spawning thread-safe actor: pid={}, name={}", pid, name);

        // Create our actor context and our actor with it.
        let (manager, sender, receiver) = inbox::Manager::new_small_channel();
        let actor_ref = ActorRef::local(sender);
        let mut ctx = actor::Context::new(receiver, ThreadSafe::new(pid, self.clone()));
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

    /// Spawn a thread-safe `future`.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn spawn_future<Fut>(&self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + Send + Sync + 'static,
    {
        self.scheduler.add_future(future, options.priority())
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

    pub(crate) fn start_trace(&self) -> Option<trace::EventTiming> {
        trace::start(&self.trace_log.as_deref())
    }

    pub(crate) fn finish_trace(
        &self,
        timing: Option<trace::EventTiming>,
        pid: ProcessId,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        trace::finish(
            self.trace_log.as_deref(),
            timing,
            pid.0 as u64,
            description,
            attributes,
        )
    }
}
