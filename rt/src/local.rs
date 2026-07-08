//! Module with local runtime internals.

use std::cell::{Cell, RefCell};
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task;
use std::time::Instant;
use std::{fmt, io};

use heph::actor_ref::{ActorGroup, ActorRef, SendError};

use crate::info::Info;
use crate::metrics::{LocalMetrics, SharedMetrics};
#[cfg(any(test, feature = "test"))]
use crate::scheduler::LocalScheduler;
use crate::setup::scheduler::{ProcessId, Scheduler, Task};
use crate::setup::timers::{TimerToken, Timers};
use crate::shared::{self, SharedRuntimeData};
use crate::spawn::options::Priority;
#[cfg(any(test, feature = "test"))]
use crate::timing_wheel::TimingWheel;
use crate::{RuntimeRef, panic_message, process, trace, worker};

/// Trait to support type erasure needed by [`RuntimeRef`].
pub(crate) trait LocalRuntimeData: fmt::Debug {
    // NOTE: these methods are documented on RuntimeRef or PrivateAccess.

    fn worker_id(&self) -> NonZeroUsize;
    fn cpu_affinity(&self) -> Option<usize>;

    /// Make [`RuntimeInternals::started`] return true.
    fn start(&self);
    /// Relay a process `signal` to all actors that wanted to receive it, or
    /// returns an error if no actors want to receive it.
    fn relay_signal(&self, signal: process::Signal);
    /// Run user function `f`, setting the error if it fails.
    // NOTE: this method should take `&Rc<Self>` (Rc by reference) because it
    // doesn't need to own it, but that makes it Dyn incompatible.
    fn run_user_function(self: Rc<Self>, f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String>>);

    fn local_sq(&self) -> a10::SubmissionQueue;
    fn receive_signals(&self, actor_ref: ActorRef<process::Signal>);

    fn add_local_timer(&self, deadline: Instant, waker: task::Waker) -> TimerToken;
    fn remove_local_timer(&self, deadline: Instant, token: TimerToken);

    fn add_local_task(&self, priority: Priority, task: Pin<Box<dyn Task>>) -> ProcessId;

    fn start_trace(&self) -> Option<trace::EventTiming>;
    fn finish_trace(
        &self,
        timing: Option<trace::EventTiming>,
        substream_id: u64,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    );

    fn info(&self) -> &Info;
    fn local_metrics(&self) -> LocalMetrics;
    fn shared_metrics(&self) -> SharedMetrics;

    fn shared_ring_pollable(&self, sq: a10::SubmissionQueue) -> a10::poll::Pollable;
    fn try_poll_shared_ring(&self) -> io::Result<()>;
    fn shared(&self) -> Arc<dyn SharedRuntimeData>;
    fn clone_shared(&self) -> Arc<shared::RuntimeInternals>;
}

/// Internals of the runtime, to which `RuntimeRef`s have a reference.
#[derive(Debug)]
pub(crate) struct RuntimeInternals<S, T> {
    /// Unique id among the worker threads.
    pub(crate) id: NonZeroUsize,
    /// Runtime internals shared between coordinator and worker threads.
    pub(crate) shared: Arc<shared::RuntimeInternals>,
    /// I/O ring.
    pub(crate) ring: RefCell<a10::Ring>,
    /// Scheduler for thread-local actors.
    pub(crate) scheduler: RefCell<S>,
    /// Timers, deadlines and timeouts.
    pub(crate) timers: RefCell<T>,
    /// Actor references to relay received process signals to.
    pub(crate) signal_receivers: RefCell<ActorGroup<process::Signal>>,
    /// CPU affinity of the worker thread, or `None` if not set.
    cpu: Option<usize>,
    /// Log used for tracing, `None` is tracing is disabled.
    pub(crate) trace_log: RefCell<Option<trace::Log>>,
    /// Whether or not the runtime was started.
    ///
    /// This is here because the worker threads are started before
    /// [`Runtime::start`] is called and thus before any actors are added to the
    /// runtime. Because of this the worker could check all schedulers, see that
    /// no actors are in them and determine it's done before even starting the
    /// runtime.
    ///
    /// [`Runtime::start`]: crate::Runtime::start
    started: Cell<bool>,
    /// Fatal error hit in one of the system actors that should stop the worker.
    error: RefCell<Option<worker::Error>>,
}

impl<S, T> RuntimeInternals<S, T>
where
    Self: LocalRuntimeData,
{
    /// Create a local runtime internals.
    pub(crate) fn new(
        id: NonZeroUsize,
        shared_internals: Arc<shared::RuntimeInternals>,
        ring: a10::Ring,
        scheduler: S,
        timers: T,
        cpu: Option<usize>,
        trace_log: Option<trace::Log>,
    ) -> RuntimeInternals<S, T> {
        RuntimeInternals {
            id,
            shared: shared_internals,
            scheduler: RefCell::new(scheduler),
            ring: RefCell::new(ring),
            timers: RefCell::new(timers),
            signal_receivers: RefCell::new(ActorGroup::empty()),
            cpu,
            trace_log: RefCell::new(trace_log),
            started: Cell::new(false),
            error: RefCell::new(None),
        }
    }

    /// Print metrics about the runtime internals.
    fn log_metrics(&self) {
        let timing = trace::start(&*self.trace_log.borrow());
        let metrics = self.local_metrics();
        log::info!(
            target: "metrics",
            worker_id = self.id.get(),
            cpu_affinity = self.cpu,
            scheduler_ready = metrics.scheduler_ready(),
            scheduler_inactive = metrics.scheduler_inactive(),
            timers = metrics.timers(),
            timers_next:? = metrics.timers_next(),
            process_signal_receivers = self.signal_receivers.borrow().len(),
            cpu_time:? = metrics.cpu_time(),
            trace_counter = metrics.trace_counter();
            "worker metrics",
        );
        trace::finish_rt(
            self.trace_log.borrow_mut().as_mut(),
            timing,
            "Printing runtime metrics",
            &[],
        );
    }

    /// Whether or not the runtime has been started.
    pub(crate) fn started(&self) -> bool {
        self.started.get()
    }

    /// Set a fatal worker error.
    fn set_err(&self, err: worker::Error) {
        let mut got_err = self.error.borrow_mut();
        // We always keep the first error.
        if got_err.is_none() {
            *got_err = Some(err);
        }
    }

    /// Take a fatal worker error, if set.
    pub(crate) fn take_err(&self) -> Option<worker::Error> {
        self.error.borrow_mut().take()
    }
}

#[cfg(any(test, feature = "test"))]
impl RuntimeInternals<LocalScheduler, TimingWheel> {
    pub(crate) fn new_test(
        shared_internals: Arc<shared::RuntimeInternals>,
    ) -> RuntimeInternals<LocalScheduler, TimingWheel> {
        let ring = a10::Ring::new().unwrap();
        let waker = worker::WorkerWaker::new(ring.sq());
        RuntimeInternals::new(
            NonZeroUsize::new(1).unwrap(),
            shared_internals,
            ring,
            LocalScheduler::new(waker),
            TimingWheel::new(),
            None,
            None,
        )
    }
}

impl<S, T> LocalRuntimeData for RuntimeInternals<S, T>
where
    S: Scheduler + 'static,
    T: Timers + 'static,
{
    fn worker_id(&self) -> NonZeroUsize {
        self.id
    }

    fn cpu_affinity(&self) -> Option<usize> {
        self.cpu
    }

    fn start(&self) {
        self.started.set(true);
    }

    fn relay_signal(&self, signal: process::Signal) {
        let timing = trace::start(&*self.trace_log.borrow());
        log::trace!(worker_id = self.id, signal:?; "received process signal");

        if let process::Signal::USER2 = signal {
            self.log_metrics();
        }

        let mut receivers = self.signal_receivers.borrow_mut();
        receivers.remove_disconnected();
        match receivers.try_send_to_all(signal) {
            Err(SendError) if signal.should_exit() => {
                self.set_err(worker::Error::ProcessInterrupted);
            }
            Ok(()) | Err(SendError) => {}
        }

        trace::finish_rt(
            self.trace_log.borrow_mut().as_mut(),
            timing,
            "Relaying process signal to actors",
            &[("signal", &format_args!("{signal:?}"))],
        );
    }

    fn run_user_function(self: Rc<Self>, f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String>>) {
        let timing = trace::start(&*self.trace_log.borrow());
        let runtime_ref = RuntimeRef {
            internals: self.clone(),
        };
        let result = panic::catch_unwind(AssertUnwindSafe(move || f(runtime_ref)));
        trace::finish_rt(
            self.trace_log.borrow_mut().as_mut(),
            timing,
            "Running user function",
            &[],
        );
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => self.set_err(worker::Error::UserFunction(err.into())),
            Err(err) => {
                let msg = format!("user function panicked: {}", panic_message(&*err));
                self.set_err(worker::Error::UserFunction(msg.into()));
            }
        }
    }

    fn local_sq(&self) -> a10::SubmissionQueue {
        self.ring.borrow().sq()
    }

    fn receive_signals(&self, actor_ref: ActorRef<process::Signal>) {
        self.signal_receivers.borrow_mut().add_unique(actor_ref);
    }

    fn add_local_timer(&self, deadline: Instant, waker: task::Waker) -> TimerToken {
        log::trace!(deadline:?; "adding local timer");
        self.timers.borrow_mut().add(deadline, waker)
    }

    fn remove_local_timer(&self, deadline: Instant, token: TimerToken) {
        log::trace!(deadline:?, token:?; "removing local timer");
        self.timers.borrow_mut().remove(deadline, token);
    }

    fn add_local_task(&self, priority: Priority, task: Pin<Box<dyn Task>>) -> ProcessId {
        self.scheduler.borrow_mut().add_boxed_task(priority, task)
    }

    fn start_trace(&self) -> Option<trace::EventTiming> {
        trace::start(&*self.trace_log.borrow())
    }

    fn finish_trace(
        &self,
        timing: Option<trace::EventTiming>,
        substream_id: u64,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        trace::finish(
            (*self.trace_log.borrow_mut()).as_mut(),
            timing,
            substream_id,
            description,
            attributes,
        );
    }

    fn info(&self) -> &Info {
        self.shared.info()
    }

    fn local_metrics(&self) -> LocalMetrics {
        let scheduler = self.scheduler.borrow();
        let mut timers = self.timers.borrow_mut();
        LocalMetrics {
            scheduler_ready: scheduler.processes_ready(),
            scheduler_inactive: scheduler.processes_inactive(),
            timers: timers.len(),
            timers_next: timers.until_next_deadline(),
            trace_counter: self
                .trace_log
                .borrow()
                .as_ref()
                .map_or(0, |t| t.counter() as usize),
        }
    }

    fn shared_metrics(&self) -> SharedMetrics {
        self.shared.metrics()
    }

    fn shared_ring_pollable(&self, sq: a10::SubmissionQueue) -> a10::poll::Pollable {
        self.shared.ring_pollable(sq)
    }

    fn try_poll_shared_ring(&self) -> io::Result<()> {
        self.shared.try_poll_ring()
    }

    fn shared(&self) -> Arc<dyn SharedRuntimeData> {
        self.shared.clone()
    }

    fn clone_shared(&self) -> Arc<shared::RuntimeInternals> {
        self.shared.clone()
    }
}
