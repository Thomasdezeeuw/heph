//! Module with shared runtime internals.

use std::cell::{Cell, RefCell};
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::Arc;

use heph::actor_ref::{ActorGroup, SendError};
use log::{info, trace};

use crate::scheduler::Scheduler;
use crate::timers::Timers;
use crate::wakers::Wakers;
use crate::{cpu_usage, panic_message, shared, trace, worker, RuntimeRef, Signal};

/// Internals of the runtime, to which `RuntimeRef`s have a reference.
#[derive(Debug)]
pub(crate) struct RuntimeInternals {
    /// Unique id among the worker threads.
    pub(crate) id: NonZeroUsize,
    /// Runtime internals shared between coordinator and worker threads.
    pub(crate) shared: Arc<shared::RuntimeInternals>,
    /// Creation of `task::Waker`s for for thread-local actors.
    pub(crate) wakers: RefCell<Wakers>,
    /// Scheduler for thread-local actors.
    pub(crate) scheduler: RefCell<Scheduler>,
    /// io_uring completion ring.
    pub(crate) ring: RefCell<a10::Ring>,
    /// Timers, deadlines and timeouts.
    pub(crate) timers: RefCell<Timers>,
    /// Actor references to relay received `Signal`s to.
    pub(crate) signal_receivers: RefCell<ActorGroup<Signal>>,
    /// CPU affinity of the worker thread, or `None` if not set.
    pub(crate) cpu: Option<usize>,
    /// Log used for tracing, `None` is tracing is disabled.
    pub(crate) trace_log: RefCell<Option<trace::Log>>,
    /// Whether or not the runtime was started.
    ///
    /// This is here because the worker threads are started before
    /// [`crate::Runtime::start`] is called and thus before any actors are added
    /// to the runtime. Because of this the worker could check all schedulers,
    /// see that no actors are in them and determine it's done before even
    /// starting the runtime.
    ///
    /// [`Runtime::start`]: rt::Runtime::start
    started: Cell<bool>,
    /// Fatal error hit in one of the system actors that should stop the worker.
    error: RefCell<Option<worker::Error>>,
}

impl RuntimeInternals {
    /// Create a local runtime internals.
    pub(crate) fn new(
        id: NonZeroUsize,
        shared_internals: Arc<shared::RuntimeInternals>,
        wakers: Wakers,
        ring: a10::Ring,
        cpu: Option<usize>,
        trace_log: Option<trace::Log>,
    ) -> RuntimeInternals {
        RuntimeInternals {
            id,
            shared: shared_internals,
            wakers: RefCell::new(wakers),
            scheduler: RefCell::new(Scheduler::new()),
            ring: RefCell::new(ring),
            timers: RefCell::new(Timers::new()),
            signal_receivers: RefCell::new(ActorGroup::empty()),
            cpu,
            trace_log: RefCell::new(trace_log),
            started: Cell::new(false),
            error: RefCell::new(None),
        }
    }

    /// Relay a process `signal` to all actors that wanted to receive it, or
    /// returns an error if no actors want to receive it.
    pub(crate) fn relay_signal(&self, signal: Signal) {
        let timing = trace::start(&*self.trace_log.borrow());
        trace!(worker_id = self.id.get(), signal:? = signal; "received process signal");

        if let Signal::User2 = signal {
            self.log_metrics();
        }

        let mut receivers = self.signal_receivers.borrow_mut();
        receivers.remove_disconnected();
        match receivers.try_send_to_all(signal) {
            Err(SendError) if signal.should_stop() => {
                self.set_err(worker::Error::ProcessInterrupted);
            }
            Ok(()) | Err(SendError) => {}
        };

        trace::finish_rt(
            self.trace_log.borrow_mut().as_mut(),
            timing,
            "Relaying process signal to actors",
            &[("signal", &signal.as_str())],
        );
    }

    /// Print metrics about the runtime internals.
    pub(crate) fn log_metrics(&self) {
        let timing = trace::start(&*self.trace_log.borrow());
        let trace_metrics = self.trace_log.borrow().as_ref().map(trace::Log::metrics);
        let scheduler = self.scheduler.borrow();
        // NOTE: need mutable access to timers due to `Timers::next`.
        let mut timers = self.timers.borrow_mut();
        info!(
            target: "metrics",
            worker_id = self.id.get(),
            cpu_affinity = self.cpu,
            scheduler_ready = scheduler.ready(),
            scheduler_inactive = scheduler.inactive(),
            timers_total = timers.len(),
            timers_next:? = timers.next_timer(),
            process_signal_receivers = self.signal_receivers.borrow().len(),
            cpu_time:? = cpu_usage(libc::CLOCK_THREAD_CPUTIME_ID),
            trace_counter = trace_metrics.map_or(0, |m| m.counter);
            "worker metrics",
        );
        trace::finish_rt(
            self.trace_log.borrow_mut().as_mut(),
            timing,
            "Printing runtime metrics",
            &[],
        );
    }

    /// Run user function `f`, setting the error if it fails.
    pub(crate) fn run_user_function(
        self: &Rc<Self>,
        f: Box<dyn FnOnce(RuntimeRef) -> Result<(), String>>,
    ) {
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

    /// Make [`RuntimeInternals::started`] return true.
    pub(crate) fn start(&self) {
        self.started.set(true);
    }

    /// Whether or not the runtime has been started.
    pub(crate) fn started(&self) -> bool {
        self.started.get()
    }

    /// Set a fatal worker error.
    pub(crate) fn set_err(&self, err: worker::Error) {
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
