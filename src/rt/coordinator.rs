//! Coordinator thread code.

use std::io;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use log::{debug, trace};
use mio::event::Event;
use mio::{Events, Interest, Poll, Registry, Token};
use mio_signals::{SignalSet, Signals};

use crate::rt::process::ProcessId;
use crate::rt::scheduler::Scheduler;
use crate::rt::waker::{self, WakerId};
use crate::rt::{
    self, SharedRuntimeInternal, Signal, SyncWorker, Worker, SYNC_WORKER_ID_END,
    SYNC_WORKER_ID_START,
};

/// Tokens used to receive events.
const SIGNAL: Token = Token(usize::max_value());
const WAKER: Token = Token(usize::max_value() - 1);

pub(super) struct Coordinator {
    poll: Poll,
    waker_id: WakerId,
    waker_events: Receiver<ProcessId>,
    scheduler: Scheduler,
}

impl Coordinator {
    /// Initialise the `Coordinator` thread.
    pub(super) fn init() -> io::Result<(Coordinator, Arc<SharedRuntimeInternal>)> {
        let poll = Poll::new()?;
        let registry = poll.registry().try_clone()?;

        let (waker_sender, waker_events) = crossbeam_channel::unbounded();
        let waker = mio::Waker::new(&registry, WAKER)?;
        let waker_id = waker::init(waker, waker_sender);
        let scheduler = Scheduler::new();

        let shared_internals =
            SharedRuntimeInternal::new(waker_id, scheduler.create_ref(), registry);
        let coordinator = Coordinator {
            poll,
            waker_events,
            waker_id,
            scheduler,
        };

        Ok((coordinator, shared_internals))
    }

    /// Run the coordinator.
    ///
    /// # Notes
    ///
    /// `workers` must be sorted based on `id`.
    pub(super) fn run<E>(
        mut self,
        mut workers: Vec<Worker<E>>,
        mut sync_workers: Vec<SyncWorker>,
    ) -> Result<(), rt::Error<E>> {
        debug_assert!(workers.is_sorted_by_key(|w| w.id()));

        let registry = self.poll.registry();
        let mut signals = setup_signals(&registry).map_err(rt::Error::coordinator)?;
        register_workers(&registry, &mut workers).map_err(rt::Error::coordinator)?;
        register_sync_workers(&registry, &mut sync_workers).map_err(rt::Error::coordinator)?;

        let mut events = Events::with_capacity(16);
        loop {
            self.poll(&mut events).map_err(rt::Error::coordinator)?;

            let mut wake_workers = false;
            for event in events.iter() {
                trace!("event: {:?}", event);
                match event.token() {
                    SIGNAL => relay_signals(&mut signals, &mut workers, &mut sync_workers)
                        .map_err(rt::Error::coordinator)?,
                    // We always check for waker events below.
                    WAKER => {}
                    token if token.0 <= SYNC_WORKER_ID_START => {
                        handle_worker_event(&mut workers, event)?
                    }
                    token if token.0 <= SYNC_WORKER_ID_END => {
                        handle_sync_worker_event(&mut sync_workers, event)?
                    }
                    token => {
                        wake_workers = true;
                        let pid = token.into();
                        trace!("waking thread-safe actor: pid={}", pid);
                        self.scheduler.mark_ready(pid);
                    }
                }
            }

            trace!("polling wakup events");
            for pid in self.waker_events.try_iter() {
                trace!("waking thread-safe actor: pid={}", pid);
                wake_workers = true;
                self.scheduler.mark_ready(pid);
            }
            // In case the worker threads are polling we need to wake them up.
            // TODO: optimise this.
            if wake_workers {
                trace!("waking worker threads");
                for worker in workers.iter_mut() {
                    // Can't deal with the error.
                    let _ = worker.wake();
                }
            }

            if workers.is_empty() {
                return Ok(());
            }
        }
    }

    fn poll(&mut self, events: &mut Events) -> io::Result<()> {
        trace!("polling event sources");
        waker::mark_polling(self.waker_id, true);
        let res = self.poll.poll(events, None);
        waker::mark_polling(self.waker_id, false);
        res
    }
}

/// Setup a new `Signals` instance, registering it with `registry`.
fn setup_signals(registry: &Registry) -> io::Result<Signals> {
    let signals = SignalSet::all();
    trace!("setting up signals handling: signals={:?}", signals);
    Signals::new(signals).and_then(|mut signals| {
        registry
            .register(&mut signals, SIGNAL, Interest::READABLE)
            .map(|()| signals)
    })
}

/// Register all `workers`' sending end of the pipe with `registry`.
fn register_workers<E>(registry: &Registry, workers: &mut [Worker<E>]) -> io::Result<()> {
    workers
        .iter_mut()
        .map(|worker| {
            let id = worker.id();
            trace!("registering worker thread: id={}", id);
            worker.register(&registry, Token(id))
        })
        .collect()
}

/// Register all `sync_workers`' sending end of the pipe with `registry`.
fn register_sync_workers(registry: &Registry, sync_workers: &mut [SyncWorker]) -> io::Result<()> {
    sync_workers
        .iter_mut()
        .map(|worker| {
            let id = worker.id();
            trace!("registering sync actor worker thread: id={}", id);
            registry.register(worker, Token(id), Interest::WRITABLE)
        })
        .collect()
}

/// Relay all signals receive from `signals` to the `workers` and
/// `sync_workers`.
fn relay_signals<E>(
    signals: &mut Signals,
    workers: &mut [Worker<E>],
    sync_workers: &mut [SyncWorker],
) -> io::Result<()> {
    while let Some(signal) = signals.receive()? {
        debug!("received signal on coordinator: signal={:?}", signal);

        let signal = Signal::from_mio(signal);
        for worker in workers.iter_mut() {
            worker.send_signal(signal)?;
        }
        for sync_worker in sync_workers.iter_mut() {
            sync_worker.send_signal(signal);
        }
    }
    Ok(())
}

/// Handle an `event` for a worker.
fn handle_worker_event<E>(workers: &mut Vec<Worker<E>>, event: &Event) -> Result<(), rt::Error<E>> {
    if let Ok(i) = workers.binary_search_by_key(&event.token().0, |w| w.id()) {
        if event.is_error() || event.is_write_closed() {
            // Receiving end of the pipe is dropped, which means the
            // worker has shut down.
            let worker = workers.remove(i);
            debug!("worker thread done: id={}", worker.id());

            worker
                .join()
                .map_err(rt::Error::worker_panic)
                .and_then(|res| res)
        } else {
            // Sporadic event, we can ignore it.
            Ok(())
        }
    } else {
        // Sporadic event, we can ignore it.
        Ok(())
    }
}

/// Handle an `event` for a sync actor worker.
fn handle_sync_worker_event<E>(
    sync_workers: &mut Vec<SyncWorker>,
    event: &Event,
) -> Result<(), rt::Error<E>> {
    if let Ok(i) = sync_workers.binary_search_by_key(&event.token().0, |w| w.id()) {
        if event.is_error() || event.is_write_closed() {
            // Receiving end of the pipe is dropped, which means the
            // worker has shut down.
            let sync_worker = sync_workers.remove(i);
            debug!("sync actor worker thread done: id={}", sync_worker.id());

            sync_worker.join().map_err(rt::Error::sync_actor_panic)
        } else {
            Ok(())
        }
    } else {
        Ok(())
    }
}
