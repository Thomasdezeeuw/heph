use std::cell::RefCell;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{self as channel, Receiver};
use log::{debug, trace};
use mio::{event, Events, Interest, Poll, Registry, Token};
use mio_pipe::new_pipe;

use crate::system::hack::SetupFn;
use crate::system::queue::Queue;
use crate::system::scheduler::Scheduler;
use crate::system::timers::Timers;
use crate::system::{waker, ActorSystemInternal, ActorSystemRef, ProcessId, RuntimeError, Signal};

pub(super) struct Worker<E> {
    /// Unique id (among all threads in the `ActorSystem`).
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

/// Run the actor system, with an optional `setup` function.
///
/// This is the entry point for the worker threads.
fn main<S>(setup: Option<S>, receiver: mio_pipe::Receiver) -> Result<(), RuntimeError<S::Error>>
where
    S: SetupFn,
{
    let actor_system = RunningActorSystem::new(receiver).map_err(RuntimeError::worker)?;

    // Run optional setup.
    if let Some(setup) = setup {
        let system_ref = actor_system.create_ref();
        setup.setup(system_ref).map_err(RuntimeError::setup)?;
    }

    // All setup is done, so we're ready to run the event loop.
    actor_system.run_event_loop()
}

/// The system that runs all processes.
///
/// This `pub(crate)` because it's used in the test module.
#[derive(Debug)]
pub(crate) struct RunningActorSystem {
    /// Inside of the system, shared with zero or more `ActorSystemRef`s.
    internal: Rc<ActorSystemInternal>,
    events: Events,
    /// Scheduler that hold the processes, schedules and runs them.
    scheduler: Scheduler,
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

impl RunningActorSystem {
    /// Create a new running actor system.
    pub fn new(mut receiver: mio_pipe::Receiver) -> io::Result<RunningActorSystem> {
        // System queue for event notifications.
        let poll = Poll::new()?;
        let awakener = mio::Waker::new(poll.registry(), AWAKENER)?;
        poll.registry()
            .register(&mut receiver, COORDINATOR, Interest::READABLE)?;

        // Channel used in the `Waker` implementation.
        let (waker_sender, waker_recv) = channel::unbounded();
        let waker_id = waker::init(awakener, waker_sender);

        // Scheduler for scheduling and running processes.
        let (scheduler, scheduler_ref) = Scheduler::new();

        // Internals of the running actor system.
        let internal = ActorSystemInternal {
            waker_id,
            scheduler_ref: RefCell::new(scheduler_ref),
            poll: RefCell::new(poll),
            timers: RefCell::new(Timers::new()),
            queue: RefCell::new(Queue::new()),
            signal_receivers: RefCell::new(Vec::new()),
        };

        // The actor system we're going to run.
        Ok(RunningActorSystem {
            internal: Rc::new(internal),
            events: Events::with_capacity(128),
            scheduler,
            waker_events: waker_recv,
            receiver,
        })
    }

    /// Create a new reference to this actor system.
    pub fn create_ref(&self) -> ActorSystemRef {
        ActorSystemRef {
            internal: self.internal.clone(),
        }
    }

    /// Run the actor system's event loop.
    pub fn run_event_loop<E>(mut self) -> Result<(), RuntimeError<E>> {
        debug!("running actor system's event loop");

        // System reference used in running the processes.
        let mut system_ref = self.create_ref();

        loop {
            // We first run the processes and only poll after to ensure that we
            // return if there is nothing to poll, as there would be no
            // processes to run then either.
            trace!("running processes");
            for _ in 0..RUN_POLL_RATIO {
                if !self.scheduler.run_process(&mut system_ref) {
                    if self.scheduler.is_empty() {
                        // No processes left to run, so we're done.
                        debug!("no processes to run, stopping actor system");
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
        let mut check_receiver = false;
        for event in self.events.iter() {
            match event.token() {
                AWAKENER => poll_waker = true,
                COORDINATOR => check_receiver = true,
                token => self.scheduler.schedule(token.into()),
            }
        }

        if check_receiver {
            self.receive_signals()?;
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
