//! Module containing a builder for `ActorSystem`.

use std::io;

use num_cpus;
use mio_st::poll::Poll;

use system::ActorSystem;

/// A builder pattern for an [`ActorSystem`].
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorSystemBuilder {
    n_processes: usize,
}

impl ActorSystemBuilder {
    pub fn build(self) -> io::Result<ActorSystem> {
        Ok(ActorSystem {
            poll: Poll::new()?,
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
