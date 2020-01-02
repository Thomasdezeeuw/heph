//! Coordinator thread code.

use std::any::Any;

use log::{debug, trace};
use mio::{Events, Interest, Poll, Token};
use mio_signals::{SignalSet, Signals};

use crate::system::{RuntimeError, Signal, SyncWorker, Worker, SYNC_WORKER_ID_START};

const SIGNAL: Token = Token(usize::max_value());

/// Run the coordinator.
///
/// # Notes
///
/// `workers` must be sorted based on `id`.
pub(super) fn main<E>(
    mut workers: Vec<Worker<E>>,
    mut sync_workers: Vec<SyncWorker>,
) -> Result<(), RuntimeError<E>> {
    debug_assert!(workers.is_sorted_by_key(|w| w.id));

    let mut poll = Poll::new().map_err(RuntimeError::coordinator)?;
    let mut events = Events::with_capacity(16);

    let mut signals = Signals::new(SignalSet::all()).map_err(RuntimeError::coordinator)?;
    poll.registry()
        .register(&mut signals, SIGNAL, Interest::READABLE)
        .map_err(RuntimeError::coordinator)?;

    for worker in workers.iter_mut() {
        trace!("registering worker thread: {}", worker.id);
        poll.registry()
            .register(&mut worker.sender, Token(worker.id), Interest::WRITABLE)
            .map_err(RuntimeError::coordinator)?;
    }

    for sync_worker in sync_workers.iter_mut() {
        trace!("registering sync actor worker thread: {}", sync_worker.id);
        poll.registry()
            .register(
                &mut sync_worker.sender,
                Token(sync_worker.id),
                Interest::WRITABLE,
            )
            .map_err(RuntimeError::coordinator)?;
    }

    loop {
        trace!("polling on coordinator");
        poll.poll(&mut events, None)
            .map_err(RuntimeError::coordinator)?;

        for event in events.iter() {
            match event.token() {
                SIGNAL => {
                    while let Some(signal) = signals.receive().map_err(RuntimeError::coordinator)? {
                        for worker in workers.iter_mut() {
                            worker
                                .send_signal(Signal::from_mio(signal))
                                .map_err(RuntimeError::coordinator)?;
                        }
                        for sync_worker in sync_workers.iter_mut() {
                            sync_worker.send_signal(Signal::from_mio(signal));
                        }
                    }
                }
                token => {
                    if token.0 >= SYNC_WORKER_ID_START {
                        if let Ok(i) = sync_workers.binary_search_by_key(&token.0, |w| w.id) {
                            if event.is_error() || event.is_write_closed() {
                                // Receiving end of the pipe is dropped, which means the
                                // worker has shut down.
                                let sync_worker = sync_workers.remove(i);
                                debug!("sync actor worker thread {} is done", sync_worker.id);

                                sync_worker.handle.join().map_err(map_panic)?;
                            }
                        }
                    } else if let Ok(i) = workers.binary_search_by_key(&token.0, |w| w.id) {
                        if event.is_error() || event.is_write_closed() {
                            // Receiving end of the pipe is dropped, which means the
                            // worker has shut down.
                            let worker = workers.remove(i);
                            debug!("worker thread {} is done", worker.id);

                            worker
                                .handle
                                .join()
                                .map_err(map_panic)
                                .and_then(|res| res)?;
                        } else if event.is_writable() {
                            workers[i]
                                .send_pending_signals()
                                .map_err(RuntimeError::coordinator)?;
                        }
                    }
                }
            }
        }

        if workers.is_empty() {
            break;
        }
    }

    Ok(())
}

/// Maps a boxed panic messages to a `RuntimeError`.
fn map_panic<E>(err: Box<dyn Any + Send + 'static>) -> RuntimeError<E> {
    let msg = match err.downcast_ref::<&'static str>() {
        Some(s) => (*s).to_owned(),
        None => match err.downcast_ref::<String>() {
            Some(s) => s.clone(),
            None => "unkown panic message".to_owned(),
        },
    };
    RuntimeError::panic(msg)
}
