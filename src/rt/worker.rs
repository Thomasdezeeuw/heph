#![allow(dead_code, unused_variables, unused_imports)]

use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{self as channel, Receiver};
use log::{debug, trace};
use mio::{event, Events, Interest, Poll, Registry, Token};
use mio_pipe::new_pipe;

use crate::rt::hack::SetupFn;
use crate::rt::process::{Process, ProcessResult};
use crate::rt::queue::Queue;
use crate::rt::scheduler::Scheduler;
use crate::rt::timers::Timers;
use crate::rt::{waker, ProcessId, RuntimeError, RuntimeInternal, RuntimeRef, Signal};

pub(super) struct Worker<E> {
    /// Unique id (among all threads in the `Runtime`).
    id: usize,
    /// Handle for the actual thread.
    handle: thread::JoinHandle<Result<(), RuntimeError<E>>>,
    /// Sending half of the Unix pipe, used to communicate with the thread.
    sender: mio_pipe::Sender,
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
        new_pipe().and_then(|(sender, receiver)| {
            thread::Builder::new()
                .name(format!("heph_worker{}", id))
                .spawn(move || main(setup, receiver))
                .map(|handle| Worker {
                    id,
                    handle,
                    sender,
                    pending: Vec::new(),
                })
        })
    }

    /// Return the worker's id.
    pub(super) fn id(&self) -> usize {
        self.id
    }

    /// Send the worker thread a `signal`.
    ///
    /// If the signal can't be send now it will be added the to the list of
    /// pending signals, which can be send using `send_pending_signals`.
    pub(super) fn send_signal(&mut self, signal: Signal) -> io::Result<()> {
        let byte = signal.to_byte();
        loop {
            match self.sender.write(&[byte]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(_) => return Ok(()),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                    // Can't write right now, we'll do so later.
                    self.pending.push(signal);
                    return Ok(());
                }
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
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

/// Registers the sending end of the Unix pipe used to communicate with the
/// thread.
impl<E> event::Source for Worker<E> {
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

/// Run the Heph runtime, with an optional `setup` function.
///
/// This is the entry point for the worker threads.
fn main<S>(setup: Option<S>, receiver: mio_pipe::Receiver) -> Result<(), RuntimeError<S::Error>>
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
    /// Receiving side of the channel for `Waker` events.
    waker_events: Receiver<ProcessId>,
    /// Receiving end of the channel connected to the coordinator thread.
    receiver: mio_pipe::Receiver,
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
    pub(crate) fn new(mut receiver: mio_pipe::Receiver) -> io::Result<RunningRuntime> {
        // System queue for event notifications.
        let poll = Poll::new()?;
        let awakener = mio::Waker::new(poll.registry(), AWAKENER)?;
        poll.registry()
            .register(&mut receiver, COORDINATOR, Interest::READABLE)?;

        // Channel used in the `Waker` implementation.
        let (waker_sender, waker_recv) = channel::unbounded();
        let waker_id = waker::init(awakener, waker_sender);

        // Scheduler for scheduling and running processes.
        let (scheduler, _work_stealer) = Scheduler::new();
        // TODO: share the work stealer with other threads.

        // Internals of the running runtime.
        let internal = RuntimeInternal {
            waker_id,
            scheduler: RefCell::new(scheduler),
            poll: RefCell::new(poll),
            timers: RefCell::new(Timers::new()),
            queue: RefCell::new(Queue::new()),
            signal_receivers: RefCell::new(Vec::new()),
        };

        Ok(RunningRuntime {
            internal: Rc::new(internal),
            events: Events::with_capacity(128),
            waker_events: waker_recv,
            receiver,
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
                // NOTE: preferably this running of a process is handled
                // completely within a method of `Scheduler`, however this is
                // not possible.
                // Because we need `borrow_mut` the scheduler here we couldn't
                // also get a mutable reference to it to add actors, while a
                // process is running. Thus we need to remove a process from the
                // scheduler, drop the mutable reference, and only then run the
                // process. This allow a `RuntimeRef` to also mutable borrow the
                // `Scheduler` to add new actors to it.
                let process = self.internal.scheduler.borrow_mut().next_process();
                if let Some(mut process) = process {
                    match process.as_mut().run(&mut runtime_ref) {
                        ProcessResult::Complete => {}
                        ProcessResult::Pending => {
                            self.internal.scheduler.borrow_mut().add_process(process);
                        }
                    }
                } else {
                    if !self.internal.scheduler.borrow().has_process() {
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

        self.poll()?;

        let mut scheduler = self.internal.scheduler.borrow_mut();
        let mut check_coordinator = false;
        for event in self.events.iter() {
            match event.token() {
                AWAKENER => {}
                COORDINATOR => check_coordinator = true,
                token => scheduler.mark_ready(token.into()),
            }
        }

        trace!("polling wakup events");
        for pid in self.waker_events.try_iter() {
            scheduler.mark_ready(pid);
        }

        trace!("polling user space events");
        for pid in self.internal.queue.borrow_mut().events() {
            scheduler.mark_ready(pid);
        }

        trace!("polling timers");
        for pid in self.internal.timers.borrow_mut().deadlines() {
            scheduler.mark_ready(pid);
        }

        if check_coordinator {
            drop(scheduler); // Com'on rustc, this one isn't that hard...
            self.receive_signals()
        } else {
            Ok(())
        }
    }

    /// Poll for system events.
    fn poll(&mut self) -> io::Result<()> {
        let timeout = self.determine_timeout();

        // Only mark ourselves as polling if the timeout is non zero.
        let mark_waker = if !is_zero(timeout) {
            waker::mark_polling(self.internal.waker_id, true);
            true
        } else {
            false
        };

        let res = self
            .internal
            .poll
            .borrow_mut()
            .poll(&mut self.events, timeout);

        if mark_waker {
            waker::mark_polling(self.internal.waker_id, false);
        }

        res
    }

    fn determine_timeout(&self) -> Option<Duration> {
        // If there are any processes ready to run, any waker events or user
        // space events we don't want to block.
        if self.internal.scheduler.borrow().has_ready_process()
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

    fn receive_signals(&mut self) -> io::Result<()> {
        trace!("receiving process signals");
        loop {
            let mut buf = [0; 8];
            match self.receiver.read(&mut buf) {
                // Should never happen as this would mean the coordinator has
                // dropped the sending side.
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => {
                    let mut receivers = self.internal.signal_receivers.borrow_mut();
                    for signal in buf[..n].iter().copied().filter_map(Signal::from_byte) {
                        for receiver in receivers.iter_mut() {
                            // Don't care if we succeed in sending the message.
                            let _ = receiver.send(signal);
                        }
                    }
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
    }
}

/// Returns `true` is timeout is `Some(Duration::from_nanos(0))`.
fn is_zero(timeout: Option<Duration>) -> bool {
    match timeout {
        Some(timeout) if timeout.subsec_nanos() == 0 => true,
        _ => false,
    }
}
