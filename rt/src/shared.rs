//! Module with shared runtime internals.

use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};
use std::{io, task};

use heph::actor_ref::ActorRef;
use heph::supervisor::Supervisor;
use heph::{ActorFutureBuilder, NewActor};

use crate::bitmap::AtomicBitMap;
use crate::scheduler::process::{FutureProcess, ProcessId};
use crate::scheduler::shared::{Process, Scheduler};
#[cfg(test)]
use crate::spawn::options::Priority;
use crate::spawn::{ActorOptions, FutureOptions};
use crate::timers::TimerToken;
use crate::timers::shared::Timers;
use crate::wakers::shared::Wakers;
use crate::{ThreadSafe, trace};

/// Shared internals of the runtime.
#[derive(Debug)]
pub(crate) struct RuntimeInternals {
    /// io_uring completion ring.
    ring: Mutex<a10::Ring>,
    /// Submission queue for the `ring`.
    sq: a10::SubmissionQueue,
    /// Wakers used to create [`task::Waker`]s for thread-safe actors.
    wakers: Wakers,
    /// Scheduler for thread-safe actors.
    scheduler: Scheduler,
    /// Timers for thread-safe actors.
    timers: Timers,
    /// Shared trace log.
    ///
    /// # Notes
    ///
    /// Prefer not to use this but use [`trace::Log`] in local internals
    /// instead.
    trace_log: Option<Arc<trace::SharedLog>>,
    /// Coordinator submission queue used to wake it.
    coordinator_sq: a10::SubmissionQueue,
    /// Bitmap to indicate which (sync) worker threads have shutdown.
    worker_shutdown: OnceLock<Arc<AtomicBitMap>>,
}

/// Metrics for [`RuntimeInternals`].
#[derive(Debug)]
pub(crate) struct Metrics {
    pub(crate) scheduler_ready: usize,
    pub(crate) scheduler_inactive: usize,
    pub(crate) timers_total: usize,
    pub(crate) timers_next: Option<Duration>,
}

impl RuntimeInternals {
    /// Create new runtime internals.
    pub(crate) fn new(
        coordinator_sq: a10::SubmissionQueue,
        trace_log: Option<Arc<trace::SharedLog>>,
    ) -> io::Result<Arc<RuntimeInternals>> {
        let config = a10::Ring::config();
        #[cfg(any(target_os = "android", target_os = "linux"))]
        let config = config.attach_queue(&coordinator_sq);
        let ring = config.build()?;
        let sq = ring.sq();

        Ok(Arc::new_cyclic(|shared_internals| {
            let wakers = Wakers::new(shared_internals.clone());
            RuntimeInternals {
                ring: Mutex::new(ring),
                sq,
                wakers,
                scheduler: Scheduler::new(),
                timers: Timers::new(),
                trace_log,
                coordinator_sq,
                worker_shutdown: OnceLock::new(),
            }
        }))
    }

    /// Returns metrics about the shared scheduler and timers.
    pub(crate) fn metrics(&self) -> Metrics {
        Metrics {
            scheduler_ready: self.scheduler.ready(),
            scheduler_inactive: self.scheduler.inactive(),
            timers_total: self.timers.len(),
            timers_next: self.timers.next_timer(),
        }
    }

    /// Returns a new [`task::Waker`] for the thread-safe actor with `pid`.
    pub(crate) fn new_task_waker(&self, pid: ProcessId) -> task::Waker {
        self.wakers.new_task_waker(pid)
    }

    pub(crate) fn ring_pollable(&self, sq: a10::SubmissionQueue) -> a10::poll::Pollable {
        self.ring.lock().unwrap().pollable(sq)
    }

    /// Polls the io_uring completion ring if it's currently not being polled.
    pub(crate) fn try_poll_ring(&self) -> io::Result<()> {
        match self.ring.try_lock() {
            Ok(mut ring) => ring.poll(Some(Duration::ZERO)),
            Err(TryLockError::WouldBlock) => Ok(()),
            Err(TryLockError::Poisoned(err)) => panic!("failed to lock shared io_uring: {err}"),
        }
    }

    /// Returns the io_uring submission queue.
    pub(crate) const fn sq(&self) -> &a10::SubmissionQueue {
        &self.sq
    }

    /// Add a timer.
    ///
    /// See [`Timers::add`].
    pub(crate) fn add_timer(&self, deadline: Instant, waker: task::Waker) -> TimerToken {
        log::trace!(deadline:?; "adding timer");
        self.timers.add(deadline, waker)
    }

    /// Remove a previously set timer.
    ///
    /// See [`Timers::remove`].
    pub(crate) fn remove_timer(&self, deadline: Instant, token: TimerToken) {
        log::trace!(deadline:?; "removing timer");
        self.timers.remove(deadline, token);
    }

    /// Wake all futures who's timers has expired.
    ///
    /// See [`Timers::expire_timers`].
    pub(crate) fn expire_timers(&self, now: Instant) -> usize {
        self.timers.expire_timers(now)
    }

    /// Determine the timeout to use in polling based on the current time
    /// (`now`), the `current` timeout and the next deadline in the shared
    /// timers.
    ///
    /// If there are no timers this will return `current`. If `current` is
    /// smaller than the next deadline in the timers this will also
    /// return `current`. Otherwise this will return a timeout based on the
    /// next deadline.
    pub(crate) fn next_timeout(&self, now: Instant, current: Option<Duration>) -> Option<Duration> {
        match self.timers.next() {
            Some(deadline) => match deadline.checked_duration_since(now) {
                // Timer has already expired, so no blocking.
                None => Some(Duration::ZERO),
                Some(timeout) => match current {
                    Some(current) if current < timeout => Some(current),
                    Some(..) | None => Some(timeout),
                },
            },
            None => current,
        }
    }

    /// Spawn a thread-safe actor.
    #[allow(clippy::needless_pass_by_value)] // For `ActorOptions`.
    pub(crate) fn try_spawn<S, NA>(
        self: &Arc<Self>,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<RuntimeAccess = ThreadSafe> + Sync + Send + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        let rt = ThreadSafe::new(self.clone());
        let (process, actor_ref) = ActorFutureBuilder::new()
            .with_rt(rt)
            .with_inbox_size(options.inbox_size())
            .try_build(supervisor, new_actor, arg)?;
        let pid = self.scheduler.add_new_process(options.priority(), process);
        let name = NA::name();
        log::debug!(pid, name; "spawning thread-safe actor");
        Ok(actor_ref)
    }

    /// Spawn a thread-safe `future`.
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn spawn_future<Fut>(&self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + Send + Sync + 'static,
    {
        let process = FutureProcess(future);
        let pid = self.scheduler.add_new_process(options.priority(), process);
        log::debug!(pid; "spawning thread-safe future");
    }

    /// Add a new proces to the scheduler.
    #[cfg(test)]
    pub(crate) fn add_new_process<P>(&self, priority: Priority, process: P) -> ProcessId
    where
        P: crate::scheduler::process::Run + Send + Sync + 'static,
    {
        self.scheduler.add_new_process(priority, process)
    }

    /// See [`Scheduler::mark_ready`].
    pub(crate) fn mark_ready(&self, pid: ProcessId) {
        self.scheduler.mark_ready(pid);
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
    pub(crate) fn remove_process(&self) -> Option<Pin<Box<Process>>> {
        self.scheduler.remove()
    }

    /// See [`Scheduler::add_back_process`].
    pub(crate) fn add_back_process(&self, process: Pin<Box<Process>>) {
        self.scheduler.add_back_process(process);
    }

    /// See [`Scheduler::complete`].
    pub(crate) fn complete(&self, process: Pin<Box<Process>>) {
        self.scheduler.complete(process);
    }

    pub(crate) fn worker_trace_log(&self, worker_id: NonZeroUsize) -> Option<trace::Log> {
        self.trace_log
            .as_ref()
            .map(|t| t.new_stream(worker_id.get() as u32))
    }

    pub(crate) fn start_trace(&self) -> Option<trace::EventTiming> {
        trace::start(&self.trace_log.as_deref())
    }

    pub(crate) fn finish_trace(
        &self,
        timing: Option<trace::EventTiming>,
        substream_id: u64,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        trace::finish(
            self.trace_log.as_deref(),
            timing,
            substream_id,
            description,
            attributes,
        );
    }

    /// MUST only be used by the coordinator.
    pub(crate) fn set_shutdown_bitmap(&self, bitmap: Arc<AtomicBitMap>) {
        let _ = self.worker_shutdown.set(bitmap);
    }

    /// Notify the coordinator that a (sync) worker stopped.
    pub(crate) fn notify_worker_stop(&self, worker_id: NonZeroUsize) {
        log::trace!(worker_id; "notifying worker thread stopped");
        self.worker_shutdown.wait().set(worker_id.get());
        self.coordinator_sq.wake();
    }
}
