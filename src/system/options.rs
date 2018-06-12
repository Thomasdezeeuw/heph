//! Options for adding an actor to the `ActorSystem`.

pub use system::scheduler::Priority;

/// Options for adding an actor to the [`ActorSystem`].
///
/// [`ActorSystem`]: ../struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorOptions {
    /// Priority for the actor in scheduler.
    pub priority: Priority,
    /// Reserved for future expansion.
    _priv: (),
}

impl Default for ActorOptions {
    fn default() -> ActorOptions {
        ActorOptions {
            priority: Priority::default(),
            _priv: (),
        }
    }
}
