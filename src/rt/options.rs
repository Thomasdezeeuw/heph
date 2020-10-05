//! Options for adding an `Actor` to a [`Runtime`].
//!
//! [`Runtime`]: crate::Runtime

pub use crate::rt::scheduler::Priority;

/// Options for adding an actor to a [`Runtime`].
///
/// [`Runtime`]: crate::Runtime
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph::rt::ActorOptions;
///
/// let opts = ActorOptions::default();
/// # drop(opts); // Silence unused variable warning.
/// ```
///
/// Giving an actor a high priority.
///
/// ```
/// use heph::rt::options::{ActorOptions, Priority};
///
/// let opts = ActorOptions::default().with_priority(Priority::HIGH);
/// # drop(opts); // Silence unused variable warning.
/// ```
#[derive(Clone, Debug)]
pub struct ActorOptions {
    priority: Priority,
    ready: bool,
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

    /// Returns `true` if the actor is ready to run when spawned.
    ///
    /// See [`mark_ready`] for more information.
    ///
    /// [`mark_ready`]: ActorOptions::mark_ready
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// This option will mark the actor as ready to run when spawned.
    ///
    /// By default newly spawned actors will wait for an external event before
    /// they start running. This external event can for example be a message
    /// send to them, or a [`TcpStream`] becoming ready to read or write.
    ///
    /// [`TcpStream`]: crate::net::TcpStream
    pub const fn mark_ready(mut self) -> Self {
        self.ready = true;
        self
    }
}

impl Default for ActorOptions {
    fn default() -> ActorOptions {
        ActorOptions {
            priority: Priority::default(),
            ready: false,
        }
    }
}

/// Options for adding an synchronous actor to a [`Runtime`].
///
/// [`Runtime`]: crate::Runtime
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph::rt::SyncActorOptions;
///
/// let opts = SyncActorOptions::default();
/// # drop(opts); // Silence unused variable warning.
/// ```
///
/// Setting the name of the thread that runs the synchronous actor.
///
/// ```
/// use heph::rt::SyncActorOptions;
///
/// let opts = SyncActorOptions::default().with_name("My sync actor".to_owned());
/// # drop(opts); // Silence unused variable warning.
/// ```
#[derive(Debug)]
pub struct SyncActorOptions {
    pub(super) thread_name: Option<String>,
}

impl SyncActorOptions {
    /// Returns the name of thread if any.
    pub fn name(&self) -> Option<&str> {
        self.thread_name.as_deref()
    }

    /// Set the name of the thread.
    ///
    /// Defaults to `Sync actor $n`, where `$n` is some number.
    pub fn with_name(mut self, thread_name: String) -> Self {
        self.thread_name = Some(thread_name);
        self
    }
}

impl Default for SyncActorOptions {
    fn default() -> SyncActorOptions {
        SyncActorOptions { thread_name: None }
    }
}
