use num_cpus;

use system::ActorSystem;

/// A builder pattern for an [`ActorSystem`].
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorSystemBuilder {
    n_processes: usize,
}

impl ActorSystemBuilder {
    pub fn build(self) -> ActorSystem {
        ActorSystem {
        }
    }
}

impl Default for ActorSystemBuilder {
    fn default() -> ActorSystemBuilder {
        ActorSystemBuilder {
            n_processes: num_cpus::get(),
        }
    }
}
