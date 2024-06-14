//! Access to the runtime.
//!
//! Various types within this crate need access to the runtime internals to
//! handle things such as I/O or timers. Various runtimes opt to make this
//! implicit by using things such as global or thread-local data. Heph-rt
//! however opts to make this explicit by using the [`rt::Access`] trait.
//!
//! This is reflected in a number of places:
//!  * In actors in the [`NewActor::RuntimeAccess`] and
//!    [`SyncActor::RuntimeAccess`] types.
//!  * In the `RT` type [`actor::Context`] and [`sync::Context`].
//!  * In various types and function that require runtime access as an argument,
//!    for example [`TcpStream::connect`].
//!
//! This runtime access is defined by the [`Access`] trait, commonly referred to
//! as the `rt::Access` (read runtime access) trait. It comes in the following
//! kinds:
//!  * [`ThreadLocal`]: passed to thread-local actors and gives access to the
//!    local runtime.
//!  * [`ThreadSafe`]: passed to thread-safe actors and gives access to the
//!    runtime parts that are shared between threads.
//!
//! Finally we have [`Sync`], which is passed to synchronous actors and also
//! gives access to the runtime parts that are shared between threads. However
//! it doesn't actually implement the `Access` trait as synchronous actors can
//! block.
//!
//! [`rt::Access`]: crate::Access
//! [`SyncActor::RuntimeAccess`]: heph::sync::SyncActor::RuntimeAccess
//! [`TcpStream::connect`]: crate::net::TcpStream::connect

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Instant;
use std::{fmt, task};

use heph::{actor, sync, ActorRef, NewActor, Supervisor};

use crate::spawn::{ActorOptions, FutureOptions, Spawn};
use crate::timers::TimerToken;
use crate::trace::{self, Trace};
use crate::{shared, Runtime, RuntimeRef};

/// Runtime Access Trait.
///
/// This trait is used to indicate an API needs access to the Heph runtime. It
/// is used by various API to get access to the runtime, but its only usable
/// inside the Heph crate.
///
/// Also see [`NewActor::RuntimeAccess`] and [`SyncActor::RuntimeAccess`].
///
/// [`SyncActor::RuntimeAccess`]: heph::sync::SyncActor::RuntimeAccess
///
/// # Notes
///
/// This trait can't be implemented by types outside of the Heph crate.
pub trait Access: PrivateAccess {}

mod private {
    use std::task;
    use std::time::Instant;

    use crate::timers::TimerToken;
    use crate::trace;

    /// Actual trait behind [`rt::Access`].
    ///
    /// [`rt::Access`]: crate::Access
    pub trait PrivateAccess {
        /// Get access to the `SubmissionQueue`.
        fn submission_queue(&self) -> a10::SubmissionQueue;

        /// Add a new timer expiring at `deadline` waking `waker`.
        fn add_timer(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken;

        /// Remove a previously set timer.
        fn remove_timer(&mut self, deadline: Instant, token: TimerToken);

        /// Returns the CPU the thread is bound to, if any.
        fn cpu(&self) -> Option<usize>;

        /// Start timing an event if tracing is enabled, see [`trace::start`].
        fn start_trace(&self) -> Option<trace::EventTiming>;

        /// Finish tracing an event, see [`trace::finish`].
        fn finish_trace(
            &mut self,
            timing: Option<trace::EventTiming>,
            substream_id: u64,
            description: &str,
            attributes: &[(&str, &dyn trace::AttributeValue)],
        );
    }
}

pub(crate) use private::PrivateAccess;

impl<T: Access> Access for &mut T {}

impl<T: PrivateAccess> PrivateAccess for &mut T {
    fn submission_queue(&self) -> a10::SubmissionQueue {
        (**self).submission_queue()
    }

    fn add_timer(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken {
        (**self).add_timer(deadline, waker)
    }

    fn remove_timer(&mut self, deadline: Instant, token: TimerToken) {
        (**self).remove_timer(deadline, token);
    }

    fn cpu(&self) -> Option<usize> {
        (**self).cpu()
    }

    fn start_trace(&self) -> Option<trace::EventTiming> {
        (**self).start_trace()
    }

    fn finish_trace(
        &mut self,
        timing: Option<trace::EventTiming>,
        substream_id: u64,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        (**self).finish_trace(timing, substream_id, description, attributes);
    }
}

/// Provides access to the thread-local parts of the runtime.
///
/// This implements the [`Access`] trait, which is required by various APIs to
/// get access to the runtime. It also implements [`Spawn`] to spawn both
/// thread-local and thread-safe actors. Furthermore this gives access to a
/// [`RuntimeRef`] for users.
///
/// This is usually a part of the [`actor::Context`], see it for more
/// information.
///
/// This is an optimised version of [`ThreadSafe`], but doesn't allow the actor
/// to move between threads.
///
/// [`actor::Context`]: heph::actor::Context
#[derive(Clone)]
pub struct ThreadLocal {
    rt: RuntimeRef,
}

impl ThreadLocal {
    pub(crate) const fn new(rt: RuntimeRef) -> ThreadLocal {
        ThreadLocal { rt }
    }
}

impl From<RuntimeRef> for ThreadLocal {
    fn from(rt: RuntimeRef) -> ThreadLocal {
        ThreadLocal::new(rt)
    }
}

impl Deref for ThreadLocal {
    type Target = RuntimeRef;

    fn deref(&self) -> &Self::Target {
        &self.rt
    }
}

impl DerefMut for ThreadLocal {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.rt
    }
}

impl Access for ThreadLocal {}

impl PrivateAccess for ThreadLocal {
    fn submission_queue(&self) -> a10::SubmissionQueue {
        self.rt.internals.ring.borrow().submission_queue().clone()
    }

    fn add_timer(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken {
        self.rt.add_timer(deadline, waker)
    }

    fn remove_timer(&mut self, deadline: Instant, token: TimerToken) {
        self.rt.remove_timer(deadline, token);
    }

    fn cpu(&self) -> Option<usize> {
        self.rt.cpu()
    }

    fn start_trace(&self) -> Option<trace::EventTiming> {
        self.rt.start_trace()
    }

    fn finish_trace(
        &mut self,
        timing: Option<trace::EventTiming>,
        substream_id: u64,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        self.rt
            .finish_trace(substream_id, timing, description, attributes);
    }
}

impl<S, NA> Spawn<S, NA, ThreadLocal> for ThreadLocal
where
    S: Supervisor<NA> + 'static,
    NA: NewActor<RuntimeAccess = ThreadLocal> + 'static,
    NA::Actor: 'static,
{
    fn try_spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = ThreadLocal>,
    {
        Spawn::try_spawn(&mut self.rt, supervisor, new_actor, arg, options)
    }
}

impl<S, NA> Spawn<S, NA, ThreadSafe> for ThreadLocal
where
    S: Supervisor<NA> + Send + std::marker::Sync + 'static,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + std::marker::Sync + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    NA::Message: Send,
{
    fn try_spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = ThreadSafe>,
    {
        self.rt.try_spawn(supervisor, new_actor, arg, options)
    }
}

impl fmt::Debug for ThreadLocal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThreadLocal")
    }
}

/// Provides access to the thread-safe parts of the runtime.
///
/// This implements the [`Access`] trait, which is required by various APIs to
/// get access to the runtime, and implements [`Spawn`] to spawn thread-safe
/// actors. Furthermore it can spawn thread-safe [`Future`]s used
/// [`spawn_future`].
///
/// This is usually a part of the [`actor::Context`], see it for more
/// information.
///
/// [`actor::Context`]: heph::actor::Context
/// [`spawn_future`]: ThreadSafe::spawn_future
#[derive(Clone)]
pub struct ThreadSafe {
    rt: Arc<shared::RuntimeInternals>,
}

impl ThreadSafe {
    pub(crate) const fn new(rt: Arc<shared::RuntimeInternals>) -> ThreadSafe {
        ThreadSafe { rt }
    }

    /// Spawn a thread-safe [`Future`].
    ///
    /// See [`RuntimeRef::spawn_future`] for more documentation.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn spawn_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + Send + std::marker::Sync + 'static,
    {
        self.rt.spawn_future(future, options);
    }
}

impl From<&Runtime> for ThreadSafe {
    fn from(rt: &Runtime) -> ThreadSafe {
        ThreadSafe::new(rt.internals.clone())
    }
}

impl From<&RuntimeRef> for ThreadSafe {
    fn from(rt: &RuntimeRef) -> ThreadSafe {
        ThreadSafe::new(rt.clone_shared())
    }
}

impl Access for ThreadSafe {}

impl PrivateAccess for ThreadSafe {
    fn submission_queue(&self) -> a10::SubmissionQueue {
        self.rt.submission_queue().clone()
    }

    fn add_timer(&mut self, deadline: Instant, waker: task::Waker) -> TimerToken {
        self.rt.add_timer(deadline, waker)
    }

    fn remove_timer(&mut self, deadline: Instant, token: TimerToken) {
        self.rt.remove_timer(deadline, token);
    }

    fn cpu(&self) -> Option<usize> {
        None
    }

    fn start_trace(&self) -> Option<trace::EventTiming> {
        self.rt.start_trace()
    }

    fn finish_trace(
        &mut self,
        timing: Option<trace::EventTiming>,
        substream_id: u64,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        self.rt
            .finish_trace(timing, substream_id, description, attributes);
    }
}

impl<S, NA> Spawn<S, NA, ThreadSafe> for ThreadSafe
where
    S: Supervisor<NA> + Send + std::marker::Sync + 'static,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + std::marker::Sync + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    NA::Message: Send,
{
    fn try_spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = ThreadSafe>,
    {
        self.rt.try_spawn(supervisor, new_actor, arg, options)
    }
}

impl fmt::Debug for ThreadSafe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThreadSafe")
    }
}

impl<M, RT> Trace for actor::Context<M, RT>
where
    RT: Access,
{
    fn start_trace(&self) -> Option<trace::EventTiming> {
        self.runtime_ref().start_trace()
    }

    fn finish_trace(
        &mut self,
        timing: Option<trace::EventTiming>,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        let substream_id = self.pid() as u64;
        self.runtime()
            .finish_trace(timing, substream_id, description, attributes);
    }
}

/// Provides access to the thread-safe parts of the runtime for synchronous
/// actors.
///
/// It implements [`Spawn`] to spawn new thread-safe actors and [`spawn_future`]
/// to spawn thread-safe [`Future`]s.
///
/// This is usually a part of the actor's [`sync::Context`], see it for more
/// information.
///
/// [`spawn_future`]: Sync::spawn_future
#[derive(Clone)]
pub struct Sync {
    rt: Arc<shared::RuntimeInternals>,
    trace_log: Option<trace::Log>,
}

impl Sync {
    pub(crate) const fn new(
        rt: Arc<shared::RuntimeInternals>,
        trace_log: Option<trace::Log>,
    ) -> Sync {
        Sync { rt, trace_log }
    }

    /// Spawn a thread-safe [`Future`].
    ///
    /// See [`RuntimeRef::spawn_future`] for more documentation.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn spawn_future<Fut>(&mut self, future: Fut, options: FutureOptions)
    where
        Fut: Future<Output = ()> + Send + std::marker::Sync + 'static,
    {
        self.rt.spawn_future(future, options);
    }
}

impl<S, NA> Spawn<S, NA, ThreadSafe> for Sync
where
    S: Supervisor<NA> + Send + std::marker::Sync + 'static,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + std::marker::Sync + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    NA::Message: Send,
{
    fn try_spawn(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        S: Supervisor<NA>,
        NA: NewActor<RuntimeAccess = ThreadSafe>,
    {
        self.rt.try_spawn(supervisor, new_actor, arg, options)
    }
}

impl fmt::Debug for Sync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Sync")
    }
}

impl<M> Trace for sync::Context<M, Sync> {
    fn start_trace(&self) -> Option<trace::EventTiming> {
        trace::start(&self.runtime_ref().trace_log)
    }

    fn finish_trace(
        &mut self,
        timing: Option<trace::EventTiming>,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        trace::finish_rt(
            self.runtime().trace_log.as_mut(),
            timing,
            description,
            attributes,
        );
    }
}
