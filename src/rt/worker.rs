use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use std::{io, thread};

use crossbeam_channel::{self, Receiver};
use log::{debug, trace};
use mio::{Events, Poll, Registry, Token};

use crate::rt::hack::SetupFn;
use crate::rt::queue::Queue;
use crate::rt::scheduler::Scheduler;
use crate::rt::timers::Timers;
use crate::rt::{channel, waker, ProcessId, RuntimeError, RuntimeInternal, RuntimeRef, Signal};

/// Message send by the coordinator thread.
#[derive(Debug)]
pub(crate) enum CoordinatorMessage {
    /// Process received a signal.
    Signal(Signal),
}

/// Message send by the worker thread.
#[derive(Debug)]
pub(crate) enum WorkerMessage {}

pub(super) struct Worker<E> {
    /// Unique id (among all threads in the `Runtime`).
    id: usize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<Result<(), RuntimeError<E>>>,
    /// Two-way communication channel to share messages with the worker thread.
    channel: channel::Handle<CoordinatorMessage, WorkerMessage>,
    /// Any signal that could not be send (via `sender`) without blocking when
    /// `send_signal` was called previously. These will be send again when
    /// `send_pending_signals` is called.
    pending: Vec<Signal>,
}

impl<E> Worker<E> {
    /// Start a new worker thread.
    pub(super) fn start<S>(id: usize, setup: Option<S>) -> io::Result<Worker<S::Error>>
    where
        S: SetupFn<Error = E>,
        E: Send + 'static,
    {
        channel::new().and_then(|(channel, worker_handle)| {
            thread::Builder::new()
                .name(format!("heph_worker{}", id))
                .spawn(move || main(setup, worker_handle))
                .map(|handle| Worker {
                    id,
                    handle,
                    channel,
                    pending: Vec::new(),
                })
        })
    }

    /// Return the worker's id.
    pub(super) fn id(&self) -> usize {
        self.id
    }

    pub(super) fn register(&mut self, registry: &Registry, token: Token) -> io::Result<()> {
        self.channel.register(registry, token)
    }

    /// Send the worker thread a `signal`.
    ///
    /// If the signal can't be send now it will be added the to the list of
    /// pending signals, which can be send using `send_pending_signals`.
    pub(super) fn send_signal(&mut self, signal: Signal) -> io::Result<()> {
        let msg = CoordinatorMessage::Signal(signal);
        self.channel.try_send(msg)
    }

    /// Send the worker thread any pending signals.
    pub(super) fn send_pending_signals(&mut self) -> io::Result<()> {
        while let Some(signal) = self.pending.first().copied() {
            self.send_signal(signal)?;
            let _ = self.pending.remove(0);
        }
        Ok(())
    }

    /// See [`thread::JoinHandle::join`].
    pub(super) fn join(self) -> thread::Result<Result<(), RuntimeError<E>>> {
        self.handle.join()
    }
}

/// Run the Heph runtime, with an optional `setup` function.
///
/// This is the entry point for the worker threads.
fn main<S>(
    setup: Option<S>,
    receiver: channel::Handle<WorkerMessage, CoordinatorMessage>,
) -> Result<(), RuntimeError<S::Error>>
where
    S: SetupFn,
{
    let runtime = RunningRuntime::new(receiver).map_err(RuntimeError::worker)?;

    // Run optional setup.
    if let Some(setup) = setup {
        let runtime_ref = runtime.create_ref();
        setup.setup(runtime_ref).map_err(RuntimeError::setup)?;
    }

    // All setup is done, so we're ready to run the event loop.
    runtime.run_event_loop()
}

/// The runtime that runs all processes.
///
/// This `pub(crate)` because it's used in the test module.
#[derive(Debug)]
pub(crate) struct RunningRuntime {
    /// Inside of the runtime, shared with zero or more `RuntimeRef`s.
    internal: Rc<RuntimeInternal>,
    events: Events,
    /// Scheduler that hold the processes, schedules and runs them.
    scheduler: Scheduler,
    /// Receiving side of the channel for `Waker` events.
    waker_events: Receiver<ProcessId>,
    /// Two-way communication channel to share messages with the coordinator.
    channel: channel::Handle<WorkerMessage, CoordinatorMessage>,
}

/// Number of processes to run before polling.
///
/// This number is chosen arbitrarily, if you can improve it please do.
// TODO: find a good balance between polling, polling user space events only and
// running processes.
const RUN_POLL_RATIO: usize = 32;

/// Id used for the awakener.
const AWAKENER: Token = Token(usize::max_value());
const COORDINATOR: Token = Token(usize::max_value() - 1);

impl RunningRuntime {
    /// Create a new running runtime.
    pub(crate) fn new(
        mut channel: channel::Handle<WorkerMessage, CoordinatorMessage>,
    ) -> io::Result<RunningRuntime> {
        // System queue for event notifications.
        let poll = Poll::new()?;
        let awakener = mio::Waker::new(poll.registry(), AWAKENER)?;
        channel.register(poll.registry(), COORDINATOR)?;

        // Channel used in the `Waker` implementation.
        let (waker_sender, waker_recv) = crossbeam_channel::unbounded();
        let waker_id = waker::init(awakener, waker_sender);

        // Scheduler for scheduling and running processes.
        let (scheduler, scheduler_ref) = Scheduler::new();

        // Internals of the running runtime.
        let internal = RuntimeInternal {
            waker_id,
            scheduler_ref: RefCell::new(scheduler_ref),
            poll: RefCell::new(poll),
            timers: RefCell::new(Timers::new()),
            queue: RefCell::new(Queue::new()),
            signal_receivers: RefCell::new(Vec::new()),
        };

        Ok(RunningRuntime {
            internal: Rc::new(internal),
            events: Events::with_capacity(128),
            scheduler,
            waker_events: waker_recv,
            channel,
        })
    }

    /// Create a new reference to this runtime.
    pub(crate) fn create_ref(&self) -> RuntimeRef {
        RuntimeRef {
            internal: self.internal.clone(),
        }
    }

    /// Run the runtime's event loop.
    fn run_event_loop<E>(mut self) -> Result<(), RuntimeError<E>> {
        debug!("running runtime's event loop");

        // System reference used in running the processes.
        let mut runtime_ref = self.create_ref();

        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there is nothing to poll, as there would be no
            // processes to run then either.
            trace!("running processes");
            for _ in 0..RUN_POLL_RATIO {
                if !self.scheduler.run_process(&mut runtime_ref) {
                    if self.scheduler.is_empty() {
                        // No processes left to run, so we're done.
                        debug!("no processes to run, stopping runtime");
                        return Ok(());
                    } else {
                        // No processes ready to run, try polling again.
                        break;
                    }
                }
            }

            self.schedule_processes().map_err(RuntimeError::worker)?;
        }
    }

    /// Schedule processes.
    ///
    /// This polls all event subsystems and schedules processes based on them.
    fn schedule_processes(&mut self) -> io::Result<()> {
        trace!("polling event sources to schedule processes");

        let timeout = self.determine_timeout();

        self.internal
            .poll
            .borrow_mut()
            .poll(&mut self.events, timeout)?;

        let mut poll_waker = false;
        let mut check_coordinator_channel = false;
        for event in self.events.iter() {
            match event.token() {
                AWAKENER => poll_waker = true,
                COORDINATOR => check_coordinator_channel = true,
                token => self.scheduler.schedule(token.into()),
            }
        }

        if check_coordinator_channel {
            self.check_coordinator()?;
        }

        if poll_waker {
            // We must first mark the waker as polled and only after poll the
            // waker events to ensure we don't miss any wake ups.
            waker::mark_polled(self.internal.waker_id);

            trace!("polling wakup events");
            for pid in self.waker_events.try_iter() {
                self.scheduler.schedule(pid);
            }
        }

        trace!("polling user space events");
        for pid in self.internal.queue.borrow_mut().events() {
            self.scheduler.schedule(pid);
        }

        trace!("polling timers");
        for pid in self.internal.timers.borrow_mut().deadlines() {
            self.scheduler.schedule(pid);
        }

        Ok(())
    }

    /// Determine the timeout to be used in `Poll::poll`.
    fn determine_timeout(&self) -> Option<Duration> {
        // If there are any processes ready to run, any waker events or user
        // space events we don't want to block.
        if self.scheduler.has_active_process()
            || !self.waker_events.is_empty()
            || !self.internal.queue.borrow().is_empty()
        {
            Some(Duration::from_millis(0))
        } else if let Some(deadline) = self.internal.timers.borrow().next_deadline() {
            let now = Instant::now();
            if deadline <= now {
                // Deadline has already expired, so no blocking.
                Some(Duration::from_millis(0))
            } else {
                // Time between the deadline and right now.
                Some(deadline.duration_since(now))
            }
        } else {
            None
        }
    }

    /// Check messages from the coordinator.
    fn check_coordinator(&mut self) -> io::Result<()> {
        use CoordinatorMessage::*;
        while let Some(msg) = self.channel.try_recv()? {
            match msg {
                Signal(signal) => {
                    trace!("received process signal: {:?}", signal);
                    let mut receivers = self.internal.signal_receivers.borrow_mut();
                    for receiver in receivers.iter_mut() {
                        // Don't care if we succeed in sending the message.
                        let _ = receiver.send(signal);
                    }
                }
            }
        }
        Ok(())
    }
}
