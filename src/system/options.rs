//! Options for adding an `Actor` to an `ActorSystem`.

use std::fmt;

pub use crate::system::process::Priority;

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
/// let opts = ActorOptions {
///     priority: Priority::HIGH,
///     ..Default::default()
/// };
///
/// # drop(opts); // Silence unused variable warning.
/// ```
#[derive(Clone)]
pub struct ActorOptions {
    /// Scheduling priority.
    pub priority: Priority,
    /// This option will schedule the actor to be run when added to the actor
    /// system, defaults to false.
    ///
    /// By default actors added to the actor system will wait for an external
    /// notification before they start running. This can happen for example by a
    /// message send to them, or a `TcpStream` becoming ready to read or write.
    pub schedule: bool,
    /// Reserved for future expansion. Use the `Default` implementation to set
    /// this field, see examples in structure documentation.
    #[doc(hidden)]
    pub __private: (),
}

impl Default for ActorOptions {
    fn default() -> ActorOptions {
        ActorOptions {
            priority: Priority::default(),
            schedule: false,
            __private: (),
        }
    }
}

impl fmt::Debug for ActorOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorOptions")
            .field("priority", &self.priority)
            .field("schedule", &self.schedule)
            .finish()
    }
}
