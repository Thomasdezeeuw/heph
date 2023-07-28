//! Options for [spawning] an [`Actor`], [`SyncActor`] or [`Future`].
//!
//! [spawning]: crate::spawn::Spawn
//! [`Actor`]: heph::actor::Actor
//! [`SyncActor`]: heph::sync::SyncActor
//! [`Future`]: std::future::Future

use std::cmp::Ordering;
use std::ops::Mul;
use std::time::Duration;

pub use heph::future::InboxSize;

/// Options for [spawning] an [`Actor`].
///
/// [spawning]: crate::spawn::Spawn
/// [`Actor`]: heph::actor::Actor
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph_rt::spawn::ActorOptions;
///
/// let opts = ActorOptions::default();
/// # _ = opts; // Silence unused variable warning.
/// ```
///
/// Giving an actor a high priority.
///
/// ```
/// use heph_rt::spawn::options::{ActorOptions, Priority};
///
/// let opts = ActorOptions::default().with_priority(Priority::HIGH);
/// # _ = opts; // Silence unused variable warning.
/// ```
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct ActorOptions {
    priority: Priority,
    inbox_size: InboxSize,
}

impl ActorOptions {
    /// Default options for system actors.
    pub(crate) const SYSTEM: ActorOptions = ActorOptions {
        priority: Priority::SYSTEM,
        inbox_size: InboxSize::ONE,
    };

    /// Returns the priority set in the options.
    pub const fn priority(&self) -> Priority {
        self.priority
    }

    /// Set the scheduling priority.
    pub const fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Returns the inbox size set in the options.
    pub const fn inbox_size(&self) -> InboxSize {
        self.inbox_size
    }

    /// Set the inbox size.
    pub const fn with_inbox_size(mut self, inbox_size: InboxSize) -> Self {
        self.inbox_size = inbox_size;
        self
    }
}

/// Priority for an actor or future in the scheduler.
///
/// Actors and futures with a higher priority will be scheduled to run more
/// often and quicker (after they return [`Poll::Pending`]) then actors/futures
/// with a lower priority.
///
/// [`Poll::Pending`]: std::task::Poll::Pending
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Priority(u8);

impl Priority {
    /// Low priority.
    ///
    /// Other actors have priority over this actor.
    pub const LOW: Priority = Priority(15);

    /// Normal priority.
    ///
    /// Most actors should run at this priority, hence its also the default
    /// priority.
    pub const NORMAL: Priority = Priority(10);

    /// High priority.
    ///
    /// Takes priority over other actors.
    pub const HIGH: Priority = Priority(5);

    /// System priority.
    pub(crate) const SYSTEM: Priority = Priority(0);
}

impl Default for Priority {
    fn default() -> Priority {
        Priority::NORMAL
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.cmp(&self.0)
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }

    fn lt(&self, other: &Self) -> bool {
        other.0 < self.0
    }

    fn le(&self, other: &Self) -> bool {
        other.0 <= self.0
    }

    fn gt(&self, other: &Self) -> bool {
        other.0 > self.0
    }

    fn ge(&self, other: &Self) -> bool {
        other.0 >= self.0
    }
}

/// Implementation detail, please ignore.
#[doc(hidden)]
impl Mul<Priority> for Duration {
    type Output = Duration;

    fn mul(self, rhs: Priority) -> Duration {
        self * u32::from(rhs.0)
    }
}

#[test]
fn priority_duration_multiplication() {
    let duration = Duration::from_millis(1);
    let system = duration * Priority::SYSTEM;
    let high = duration * Priority::HIGH;
    let normal = duration * Priority::NORMAL;
    let low = duration * Priority::LOW;

    assert!(system < high);
    assert!(system < normal);
    assert!(system < low);
    assert!(high < normal);
    assert!(high < low);
    assert!(normal < low);
}

/// Options for spawning a [`SyncActor`].
///
/// [`SyncActor`]: heph::sync::SyncActor
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph_rt::spawn::SyncActorOptions;
///
/// let opts = SyncActorOptions::default();
/// # _ = opts; // Silence unused variable warning.
/// ```
///
/// Setting the name of the thread that runs the synchronous actor.
///
/// ```
/// use heph_rt::spawn::SyncActorOptions;
///
/// let opts = SyncActorOptions::default().with_thread_name("My sync actor".to_owned());
/// # _ = opts; // Silence unused variable warning.
/// ```
#[derive(Debug, Default)]
#[must_use]
pub struct SyncActorOptions {
    thread_name: Option<String>,
    inbox_size: InboxSize,
}

impl SyncActorOptions {
    /// Returns the thread name, if any.
    pub fn thread_name(&self) -> Option<&str> {
        self.thread_name.as_deref()
    }

    /// Removes the name.
    pub(crate) fn take_thread_name(self) -> Option<String> {
        self.thread_name
    }

    /// Set the name of the thread running the actor.
    ///
    /// Defaults to the name of the actor, this can be used if multiple
    /// synchronous actors of the same kind are started and you want to give
    /// them unique names.
    pub fn with_thread_name(mut self, thread_name: String) -> Self {
        self.thread_name = Some(thread_name);
        self
    }

    /// Returns the inbox size set in the options.
    pub const fn inbox_size(&self) -> InboxSize {
        self.inbox_size
    }

    /// Set the inbox size.
    pub const fn with_inbox_size(mut self, inbox_size: InboxSize) -> Self {
        self.inbox_size = inbox_size;
        self
    }
}

/// Options for spawning a [`Future`].
///
/// [`Future`]: std::future::Future
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph_rt::spawn::FutureOptions;
///
/// let opts = FutureOptions::default();
/// # _ = opts; // Silence unused variable warning.
/// ```
///
/// Giving the future a high priority.
///
/// ```
/// use heph_rt::spawn::options::{FutureOptions, Priority};
///
/// let opts = FutureOptions::default().with_priority(Priority::HIGH);
/// # _ = opts; // Silence unused variable warning.
/// ```
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct FutureOptions {
    priority: Priority,
}

impl FutureOptions {
    /// Returns the priority set in the options.
    pub const fn priority(&self) -> Priority {
        self.priority
    }

    /// Set the scheduling priority.
    pub const fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }
}
