//! Module containing a builder for `ActorSystem`.

use std::io;
use std::cell::RefCell;
use std::rc::Rc;

use num_cpus;
use mio_st::poll::Poll;

use system::{ActorSystem, ActorSystemInner};
use system::process::ProcessIdGenerator;
use system::scheduler::Scheduler;

/// A builder pattern for an [`ActorSystem`].
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorSystemBuilder {
    n_processes: usize,
}

impl ActorSystemBuilder {
    /// Builder the `ActorSystem`.
    pub fn build(self) -> io::Result<ActorSystem> {
        debug!("building actor system: n_processes={}",
            self.n_processes);
        let inner = ActorSystemInner {
            scheduler: Scheduler::new(),
            has_initiators: false,
            pid_gen: ProcessIdGenerator::new(),
            poll: Poll::new()?,
        };

        Ok(ActorSystem {
            inner: Rc::new(RefCell::new(inner)),
        })
    }
}

impl Default for ActorSystemBuilder {
    fn default() -> ActorSystemBuilder {
        ActorSystemBuilder {
            n_processes: num_cpus::get(),
        }
    }
}
