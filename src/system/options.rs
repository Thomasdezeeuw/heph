//! Options for adding an actor to the `ActorSystem`.

use std::fmt;

pub use system::scheduler::Priority;

/// Options for adding an actor to the [`ActorSystem`].
///
/// [`ActorSystem`]: ../struct.ActorSystem.html
pub struct ActorOptions {
    /// Priority for the actor in scheduler.
    pub priority: Priority,
    /// Reserved for future expansion.
    #[doc(hidden)]
    pub __private: (),
}

impl Default for ActorOptions {
    fn default() -> ActorOptions {
        ActorOptions {
            priority: Priority::default(),
            __private: (),
        }
    }
}

impl fmt::Debug for ActorOptions {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorOptions")
            .field("priority", &self.priority)
            .finish()
    }
}
