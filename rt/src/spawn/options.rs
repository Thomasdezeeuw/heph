//! Options for [spawning] an [`Actor`], [`SyncActor`] or [`Future`].
//!
//! [spawning]: crate::spawn::Spawn
//! [`Actor`]: heph::actor::Actor
//! [`SyncActor`]: heph::actor::SyncActor
//! [`Future`]: std::future::Future

use std::cmp::Ordering;
use std::num::NonZeroU8;
use std::ops::Mul;
use std::time::Duration;

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
/// # drop(opts); // Silence unused variable warning.
/// ```
///
/// Giving an actor a high priority.
///
/// ```
/// use heph_rt::spawn::options::{ActorOptions, Priority};
///
/// let opts = ActorOptions::default().with_priority(Priority::HIGH);
/// # drop(opts); // Silence unused variable warning.
/// ```
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct ActorOptions {
    priority: Priority,
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
pub struct Priority(NonZeroU8);

impl Priority {
    /// Low priority.
    ///
    /// Other actors have priority over this actor.
    pub const LOW: Priority = Priority(NonZeroU8::new(15).unwrap());

    /// Normal priority.
    ///
    /// Most actors should run at this priority, hence its also the default
    /// priority.
    pub const NORMAL: Priority = Priority(NonZeroU8::new(10).unwrap());

    /// High priority.
    ///
    /// Takes priority over other actors.
    pub const HIGH: Priority = Priority(NonZeroU8::new(5).unwrap());
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
        other.0.partial_cmp(&self.0)
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
        self * u32::from(rhs.0.get())
    }
}

#[test]
fn priority_duration_multiplication() {
    let duration = Duration::from_millis(1);
    let high = duration * Priority::HIGH;
    let normal = duration * Priority::NORMAL;
    let low = duration * Priority::LOW;

    assert!(high < normal);
    assert!(normal < low);
    assert!(high < low);
}

/// Options for spawning a [`SyncActor`].
///
/// [`SyncActor`]: heph::actor::SyncActor
///
/// # Examples
///
/// Using the default options.
///
/// ```
/// use heph_rt::spawn::SyncActorOptions;
///
/// let opts = SyncActorOptions::default();
/// # drop(opts); // Silence unused variable warning.
/// ```
///
/// Setting the name of the thread that runs the synchronous actor.
///
/// ```
/// use heph_rt::spawn::SyncActorOptions;
///
/// let opts = SyncActorOptions::default().with_name("My sync actor".to_owned());
/// # drop(opts); // Silence unused variable warning.
/// ```
#[derive(Debug, Default)]
#[must_use]
pub struct SyncActorOptions {
    thread_name: Option<String>,
}

impl SyncActorOptions {
    /// Returns the name of the synchronous actor, if any.
    pub fn name(&self) -> Option<&str> {
        self.thread_name.as_deref()
    }

    /// Removes the name.
    pub(crate) fn take_name(self) -> Option<String> {
        self.thread_name
    }

    /// Set the name of the actor. This is for example used in the naming of the
    /// thread in which the actor runs.
    ///
    /// Defaults to "Sync actor `$n`", where `$n` is some number.
    pub fn with_name(mut self, thread_name: String) -> Self {
        self.thread_name = Some(thread_name);
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
/// # drop(opts); // Silence unused variable warning.
/// ```
///
/// Giving the future a high priority.
///
/// ```
/// use heph_rt::spawn::options::{FutureOptions, Priority};
///
/// let opts = FutureOptions::default().with_priority(Priority::HIGH);
/// # drop(opts); // Silence unused variable warning.
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
