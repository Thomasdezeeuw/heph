//! Module containing a builder for `ActorSystem`.

use std::io;
use std::cell::RefCell;
use std::rc::Rc;

use num_cpus;
use mio_st::poll::Poller;

use scheduler::Scheduler;
use system::{ActorSystem, ActorSystemInner};

/// A builder pattern for an [`ActorSystem`].
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorSystemBuilder {
    n_processes: usize,
}

impl ActorSystemBuilder {
    /// Set the number of processes used, defaults to `1`.
    ///
    /// Most framework, or software in general, uses threads to make use of
    /// multiple cores, but we use processes.
    pub fn num_processes(&mut self, n_processes: usize) -> &mut Self {
        self.n_processes = n_processes;
        self
    }

    /// Set the number of processes to equal the number of cores.
    pub fn processes_cores(&mut self) -> &mut Self {
        self.num_processes(num_cpus::get())
    }

    /// Builder the `ActorSystem`.
    pub fn build(self) -> io::Result<ActorSystem> {
        debug!("building actor system: n_processes={}",
            self.n_processes);
        let inner = ActorSystemInner {
            scheduler: Scheduler::new(),
            poller: Poller::new()?,
        };

        Ok(ActorSystem {
            inner: Rc::new(RefCell::new(inner)),
            has_initiators: false,
        })
    }
}

impl Default for ActorSystemBuilder {
    fn default() -> ActorSystemBuilder {
        ActorSystemBuilder {
            n_processes: 1,
        }
    }
}
