//! Options for adding an `Actor` or `Initiator` to an `ActorSystem`.

use std::fmt;

pub use crate::scheduler::Priority;

/// Options for adding an actor to an [`ActorSystem`].
///
/// [`ActorSystem`]: ../struct.ActorSystem.html
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph::system::ActorOptions;
///
/// let opts = ActorOptions::default();
/// ```
///
/// Giving an actor a high priority.
///
/// ```
/// use heph::system::options::{ActorOptions, Priority};
///
/// let opts = ActorOptions {
///     priority: Priority::HIGH,
///     .. Default::default()
/// };
/// ```
#[derive(Clone)]
pub struct ActorOptions {
    /// Scheduling priority.
    pub priority: Priority,
    /// Register the actor in the [Actor Registry], defaults to false.
    ///
    /// [Actor Registry]: ../index.html#actor-registry
    pub register: bool,
    /// Reserved for future expansion. Use the `Default` implementation to set
    /// this field, see example in struct documentation.
    #[doc(hidden)]
    pub __private: (),
}

impl Default for ActorOptions {
    fn default() -> ActorOptions {
        ActorOptions {
            priority: Priority::default(),
            register: false,
            __private: (),
        }
    }
}

impl fmt::Debug for ActorOptions {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorOptions")
            .field("priority", &self.priority)
            .field("register", &self.register)
            .finish()
    }
}

/// Options for adding an [`Initiator`] to an [`ActorSystem`].
///
/// [`ActorSystem`]: ../struct.ActorSystem.html
/// [`Initiator`]: ../../initiator/trait.Initiator.html
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph::system::InitiatorOptions;
///
/// let opts = InitiatorOptions::default();
/// ```
#[derive(Clone)]
pub struct InitiatorOptions {
    /// Reserved for future expansion. Use the `Default` implementation to set
    /// this field, see example in struct documentation.
    #[doc(hidden)]
    pub __private: (),
}

impl Default for InitiatorOptions {
    fn default() -> InitiatorOptions {
        InitiatorOptions {
            __private: (),
        }
    }
}

impl fmt::Debug for InitiatorOptions {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("InitiatorOptions")
            .finish()
    }
}
