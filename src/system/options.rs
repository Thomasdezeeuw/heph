//! Options for adding an `Actor` to an `ActorSystem`.

pub use crate::system::scheduler::Priority;

/// Options for adding an actor to an [`ActorSystem`].
///
/// [`ActorSystem`]: crate::system::ActorSystem
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph::system::ActorOptions;
///
/// let opts = ActorOptions::default();
///
/// # drop(opts); // Silence unused variable warning.
/// ```
///
/// Giving an actor a high priority.
///
/// ```
/// use heph::system::options::{ActorOptions, Priority};
///
/// let opts = ActorOptions::default().with_priority(Priority::HIGH);
///
/// # drop(opts); // Silence unused variable warning.
/// ```
#[derive(Clone, Debug)]
pub struct ActorOptions {
    priority: Priority,
    schedule: bool,
}

impl ActorOptions {
    /// Returns the priority set in the options.
    pub const fn priority(&self) -> Priority {
        self.priority
    }

    /// Set the scheduling priority.
    pub const fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Returns true if the actor should be scheduled when added to the system.
    ///
    /// See [`schedule`] for more information.
    ///
    /// [`schedule`]: ActorOptions::schedule
    pub const fn should_schedule(&self) -> bool {
        self.schedule
    }

    /// This option will schedule the actor to be run when added to the actor
    /// system.
    ///
    /// By default actors added to the actor system will wait for an external
    /// event before they start running. This can happen for example by a
    /// message send to them, or a [`TcpStream`] becoming ready to read or write.
    ///
    /// [`TcpStream`]: crate::net::TcpStream
    pub const fn schedule(mut self) -> Self {
        // TODO: better name: auto_run, auto_schedule, start_running?
        self.schedule = true;
        self
    }
}

impl Default for ActorOptions {
    fn default() -> ActorOptions {
        ActorOptions {
            priority: Priority::default(),
            schedule: false,
        }
    }
}
