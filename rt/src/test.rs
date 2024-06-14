//! Testing facilities.
//!
//! This module will lazily create an single-threaded *test* runtime. Functions
//! such as [`try_spawn_local`] will use this to spawn and run actors. The
//! *test* runtime will not stop and the thread's resources are not cleaned up
//! (properly).
//!
//! Available utilities:
//!  * Blocking:
//!    * [`block_on_local_actor`]: spawns a thread-local [actor] and waits for
//!      the result.
//!    * [`block_on_actor`]: spawns a thread-safe [actor] and waits for the
//!      result.
//!    * [`block_on_future`]: spawns a `Future` and waits for the result.
//!  * Spawning actors:
//!    * [`try_spawn_local`] and [`spawn_local`]: attempt to spawn a thread-local [actor].
//!    * [`try_spawn`] and [`spawn`]: attempt to spawn a thread-safe [actor].
//!    * [`spawn_sync_actor`]: spawn a [synchronous actor].
//!  * Spawning futures:
//!    * [`spawn_local_future`]: spawn a thread-local [`Future`].
//!    * [`spawn_future`]: spawn a thread-safe [`Future`].
//!  * Waiting on spawned actors:
//!    * [`join`], [`join_many`]: wait for the actor(s) to finish running.
//!    * [`join_all`]: wait all actors in a group to finish running.
//!  * Initialising actors:
//!    * [`init_local_actor`]: initialise a thread-local actor.
//!    * [`init_actor`]: initialise a thread-safe actor.
//!  * Polling:
//!    * [`poll_actor`]: poll an [`Actor`].
//!    * [`poll_future`]: poll a [`Future`].
//!    * [`poll_next`]: poll an [`AsyncIterator`].
//!  * Miscellaneous:
//!    * [`size_of_actor`], [`size_of_actor_val`]: returns the size of an actor.
//!    * [`set_message_loss`]: set the percentage of messages lost on purpose.
//!    * [`PanicSupervisor`]: supervisor that panics when it receives an actor's
//!      error.
//!
//! [actor]: heph::actor
//! [synchronous actor]: SyncActor
//!
//! # Notes
//!
//! *This module is only available when the `test` feature is enabled*. It
//! shouldn't be enabled by default, and shouldn't end up in your production
//! binary.
//!
//! It is possible to only enable the test feature when testing by adding the
//! following to `Cargo.toml`.
//!
//! ```toml
//! [dev-dependencies.heph-rt]
//! features = ["test"]
//! ```

use std::any::Any;
use std::async_iter::AsyncIterator;
use std::future::{poll_fn, Future};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::pin::{pin, Pin};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::task::{self, Poll};
use std::time::{Duration, Instant};
use std::{fmt, io, slice, thread};

use heph::actor::{self, Actor, NewActor};
use heph::actor_ref::{ActorGroup, ActorRef};
use heph::supervisor::{Supervisor, SupervisorStrategy, SyncSupervisor};
use heph::sync::{SyncActor, SyncWaker};
use heph_inbox as inbox;
use heph_inbox::oneshot::{self, new_oneshot};

use crate::spawn::{ActorOptions, FutureOptions, SyncActorOptions};
use crate::worker::Worker;
use crate::{
    self as rt, panic_message, shared, sync_worker, worker, Runtime, RuntimeRef, Setup, Sync,
    ThreadLocal, ThreadSafe,
};

#[doc(no_inline)]
#[cfg(feature = "test")]
pub use heph::test::*;

fn test_coordinator() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        // NOTE: we do NOT signal the worker that the runtime has started, to
        // ensure that it doesn't stop before another test can use it.
        Setup::new()
            .with_name("Heph Test Runtime".to_string())
            .num_threads(1)
            .build()
            .expect("failed to build test runtime")
    })
}

pub(crate) fn shared_internals() -> Arc<shared::RuntimeInternals> {
    test_coordinator().internals.clone()
}

/// Returns a reference to a fake local runtime.
///
/// # Notes
///
/// The returned runtime reference is **not** a reference to the *test* runtime
/// as described in the module documentation.
pub(crate) fn runtime() -> RuntimeRef {
    thread_local! {
        /// Per thread runtime.
        static TEST_RT: Worker = {
            let (setup, sq) = worker::setup_test().expect("failed to setup test runtime");
            let (_, receiver) = rt::channel::new(sq).expect("failed to test runtime channel");
            Worker::setup(setup, receiver, shared_internals(), false, None)
        };
    }

    TEST_RT.with(Worker::create_ref)
}

/// Run function `f` on the *test* runtime.
fn run_on_test_runtime<F>(f: F)
where
    F: FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static,
{
    test_coordinator().workers[0]
        .send_function(Box::new(f))
        .expect("failed to communicate with the test runtime");
}

/// Run function `f` on the *test* runtime, waiting for the result.
fn run_on_test_runtime_wait<F, T>(f: F) -> T
where
    F: FnOnce(RuntimeRef) -> T + Send + 'static,
    T: Send + 'static,
{
    let (sender, mut receiver) = new_oneshot();
    let waker = SyncWaker::new();
    _ = receiver.register_waker(&waker.clone().into_waker());
    run_on_test_runtime(move |runtime_ref| {
        drop(sender.try_send(f(runtime_ref)));
        Ok(())
    });
    waker
        .block_on(receiver.recv_once())
        .expect("failed to receive result from test runtime")
}

/// Spawn `future` on the *test* runtime and wait for the result.
///
/// This is useful to test async functions and futures in synchronous tests.
///
/// If the future panics it will be caught and returned as error.
pub fn block_on_future<Fut>(future: Fut) -> Fut::Output
where
    Fut: Future + Send + 'static,
    Fut::Output: Send,
{
    let (sender, receiver) = new_oneshot();
    let waker = SyncWaker::new();
    spawn_local_future(
        async move {
            let mut future = pin!(future);
            let result = poll_fn(move |ctx| {
                match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(ctx))) {
                    Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
                    Ok(Poll::Pending) => Poll::Pending,
                    Err(panic) => Poll::Ready(Err(panic)),
                }
            })
            .await;
            assert!(
                sender.try_send(result).is_ok(),
                "failed to return future result"
            );
        },
        FutureOptions::default(),
    );
    match waker.block_on(receiver.recv_once()) {
        Some(Ok(output)) => output,
        Some(Err(panic)) => resume_unwind(panic),
        None => panic!("failed to receive the result of the future"),
    }
}

/// Spawn a thread-local actor on the *test* runtime and wait for it to
/// complete.
///
/// See the [module documentation] for more information about the *test*
/// runtime. And see the [`Spawn`] trait for more information about spawning
/// actors.
///
/// [module documentation]: crate::test
/// [`Spawn`]: crate::spawn::Spawn
///
/// # Notes
///
/// No superisor is used instead all errors and panics are returned by this
/// function.
///
/// This requires the `NewActor` (`NA`) to be [`Send`] as they are send to
/// another thread which runs the *test* runtime (and thus the actor). The actor
/// (`NA::Actor`) itself doesn't have to be `Send`.
pub fn block_on_local_actor<NA>(mut new_actor: NA, arg: NA::Argument)
where
    NA: NewActor<RuntimeAccess = ThreadLocal> + Send + 'static,
    NA::Actor: 'static,
    <NA::Actor as Actor>::Error: fmt::Display + Send,
    NA::Argument: Send,
    NA::Error: fmt::Display + Send,
{
    let (sender, mut receiver) = new_oneshot();
    let waker = SyncWaker::new();
    _ = receiver.register_waker(&waker.clone().into_waker());
    run_on_test_runtime(move |mut runtime_ref| {
        let (_, receiver) = heph_inbox::new(heph_inbox::MIN_CAP);
        let ctx = actor::Context::new(receiver, ThreadLocal::new(runtime_ref.clone()));
        let actor = match new_actor.new(ctx, arg) {
            Ok(actor) => actor,
            Err(err) => {
                _ = sender.try_send(Err(BlockOnError::Creating(err)));
                return Ok(());
            }
        };

        let future = ErrorCatcher {
            sender: Some(sender),
            actor,
        };

        runtime_ref.spawn_local_future(future, FutureOptions::default());
        Ok(())
    });
    match waker.block_on(receiver.recv_once()) {
        Some(Ok(())) => {}
        Some(Err(BlockOnError::<NA>::Creating(err))) => panic!("failed to create actor: {err}"),
        Some(Err(BlockOnError::<NA>::Running(err))) => panic!("actor hit an error: {err}"),
        Some(Err(BlockOnError::<NA>::Panic(panic))) => resume_unwind(panic),
        None => panic!("failed to wait on local actor"),
    }
}

/// Spawn a thread-safe actor on the *test* runtime and wait for it to
/// complete.
///
/// See the [module documentation] for more information about the *test*
/// runtime. And see the [`Spawn`] trait for more information about spawning
/// actors.
///
/// [module documentation]: crate::test
/// [`Spawn`]: crate::spawn::Spawn
///
/// # Notes
///
/// No superisor is used instead all errors and panics are returned by this
/// function.
///
/// This requires the `NewActor` (`NA`) to be [`Send`] as they are send to
/// another thread which runs the *test* runtime (and thus the actor). The actor
/// (`NA::Actor`) itself doesn't have to be `Send`.
pub fn block_on_actor<NA>(mut new_actor: NA, arg: NA::Argument)
where
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    <NA::Actor as Actor>::Error: fmt::Display + Send,
    NA::Argument: Send,
    NA::Error: fmt::Display + Send,
{
    let (sender, mut receiver) = new_oneshot();
    let waker = SyncWaker::new();
    _ = receiver.register_waker(&waker.clone().into_waker());
    run_on_test_runtime(move |mut runtime_ref| {
        let (_, receiver) = heph_inbox::new(heph_inbox::MIN_CAP);
        let ctx = actor::Context::new(receiver, ThreadSafe::new(runtime_ref.clone_shared()));
        let actor = match new_actor.new(ctx, arg) {
            Ok(actor) => actor,
            Err(err) => {
                _ = sender.try_send(Err(BlockOnError::Creating(err)));
                return Ok(());
            }
        };

        let future = ErrorCatcher {
            sender: Some(sender),
            actor,
        };

        runtime_ref.spawn_future(future, FutureOptions::default());
        Ok(())
    });
    match waker.block_on(receiver.recv_once()) {
        Some(Ok(())) => {}
        Some(Err(BlockOnError::<NA>::Creating(err))) => panic!("failed to create actor: {err}"),
        Some(Err(BlockOnError::<NA>::Running(err))) => panic!("actor hit an error: {err}"),
        Some(Err(BlockOnError::<NA>::Panic(panic))) => resume_unwind(panic),
        None => panic!("failed to wait on actor"),
    }
}

/// Error return by spawn an actor and waiting for the result.
enum BlockOnError<NA: NewActor> {
    /// Error creating the actor.
    Creating(NA::Error),
    /// Error running the actor.
    Running(<NA::Actor as Actor>::Error),
    /// Panic while running the actor.
    Panic(Box<dyn Any + Send + 'static>),
}

impl<NA> fmt::Debug for BlockOnError<NA>
where
    NA: NewActor,
    NA::Error: fmt::Debug,
    <NA::Actor as Actor>::Error: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BlockOnError::")?;
        match self {
            BlockOnError::Creating(err) => f.debug_tuple("Creating").field(&err).finish(),
            BlockOnError::Running(err) => f.debug_tuple("Running").field(&err).finish(),
            BlockOnError::Panic(err) => f
                .debug_tuple("Panic")
                .field(&panic_message(&**err))
                .finish(),
        }
    }
}

/// [`Future`]/[`Actor`] wrapper to catch errors and panics.
#[derive(Debug)]
struct ErrorCatcher<NA: NewActor> {
    sender: Option<oneshot::Sender<Result<(), BlockOnError<NA>>>>,
    actor: NA::Actor,
}

impl<NA: NewActor> Future for ErrorCatcher<NA> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving the actor.
        let this = unsafe { Pin::get_unchecked_mut(self) };
        // SAFETY: undoing the previous operation, still ensuring that the actor
        // is not moved.
        let mut actor = unsafe { Pin::new_unchecked(&mut this.actor) };
        let res = match catch_unwind(AssertUnwindSafe(|| actor.as_mut().try_poll(ctx))) {
            Ok(Poll::Ready(Ok(()))) => Ok(()),
            Ok(Poll::Ready(Err(err))) => Err(BlockOnError::Running(err)),
            Ok(Poll::Pending) => return Poll::Pending,
            Err(panic) => Err(BlockOnError::Panic(panic)),
        };
        _ = this.sender.take().unwrap().try_send(res);
        Poll::Ready(())
    }
}

/// Attempt to spawn a thread-local actor on the *test* runtime.
///
/// See the [module documentation] for more information about the *test*
/// runtime. And see the [`Spawn`] trait for more information about spawning
/// actors.
///
/// Consider using the [`TestSupervisor`] to ensure any errors in the spawned
/// actor will fail the test.
///
/// [module documentation]: crate::test
/// [`Spawn`]: crate::spawn::Spawn
///
/// # Notes
///
/// This requires the `Supervisor` (`S`) and `NewActor` (`NA`) to be [`Send`] as
/// they are send to another thread which runs the *test* runtime (and thus the
/// actor). The actor (`NA::Actor`) itself doesn't have to be `Send`.
pub fn try_spawn_local<S, NA>(
    supervisor: S,
    new_actor: NA,
    arg: NA::Argument,
    options: ActorOptions,
) -> Result<ActorRef<NA::Message>, NA::Error>
where
    S: Supervisor<NA> + Send + 'static,
    NA: NewActor<RuntimeAccess = ThreadLocal> + Send + 'static,
    NA::Actor: 'static,
    NA::Message: Send,
    NA::Argument: Send,
    NA::Error: Send,
{
    run_on_test_runtime_wait(move |mut runtime_ref| {
        runtime_ref.try_spawn_local(supervisor, new_actor, arg, options)
    })
}

/// Spawn a thread-local actor on the *test* runtime.
///
/// Same as [`try_spawn_local`], but this expects [`NewActor::Error`] to be `!`
/// (the never type).
pub fn spawn_local<S, NA>(
    supervisor: S,
    new_actor: NA,
    arg: NA::Argument,
    options: ActorOptions,
) -> ActorRef<NA::Message>
where
    S: Supervisor<NA> + Send + 'static,
    NA: NewActor<RuntimeAccess = ThreadLocal, Error = !> + Send + 'static,
    NA::Actor: 'static,
    NA::Message: Send,
    NA::Argument: Send,
{
    try_spawn_local(supervisor, new_actor, arg, options).unwrap()
}

/// Attempt to spawn a thread-safe actor on the *test* runtime.
///
/// See the [module documentation] for more information about the *test*
/// runtime. And see the [`Spawn`] trait for more information about spawning
/// actors.
///
/// Consider using the [`TestSupervisor`] to ensure any errors in the spawned
/// actor will fail the test.
///
/// [module documentation]: crate::test
/// [`Spawn`]: crate::spawn::Spawn
///
/// # Notes
///
/// This requires the `Supervisor` (`S`) and `NewActor` (`NA`) to be [`Send`] as
/// they are send to another thread which runs the *test* runtime (and thus the
/// actor). The actor (`NA::Actor`) itself doesn't have to be `Send`.
pub fn try_spawn<S, NA>(
    supervisor: S,
    new_actor: NA,
    arg: NA::Argument,
    options: ActorOptions,
) -> Result<ActorRef<NA::Message>, NA::Error>
where
    S: Supervisor<NA> + Send + std::marker::Sync + 'static,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + std::marker::Sync + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    NA::Message: Send,
    NA::Argument: Send,
    NA::Error: Send,
{
    run_on_test_runtime_wait(move |mut runtime_ref| {
        runtime_ref.try_spawn(supervisor, new_actor, arg, options)
    })
}

/// Attempt to spawn a thread-safe actor on the *test* runtime.
///
/// Same as [`try_spawn`], but this expects [`NewActor::Error`] to be `!` (the
/// never type).
pub fn spawn<S, NA>(
    supervisor: S,
    new_actor: NA,
    arg: NA::Argument,
    options: ActorOptions,
) -> ActorRef<NA::Message>
where
    S: Supervisor<NA> + Send + std::marker::Sync + 'static,
    NA: NewActor<RuntimeAccess = ThreadSafe, Error = !> + Send + std::marker::Sync + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    NA::Message: Send,
    NA::Argument: Send,
{
    try_spawn(supervisor, new_actor, arg, options).unwrap()
}

/// Spawn a thread-local [`Future`] on the *test* runtime.
///
/// See the [module documentation] for more information about the *test*
/// runtime.
///
/// [module documentation]: crate::test
pub fn spawn_local_future<Fut>(future: Fut, options: FutureOptions)
where
    Fut: Future<Output = ()> + Send + 'static,
{
    run_on_test_runtime(move |mut runtime_ref| {
        runtime_ref.spawn_local_future(future, options);
        Ok(())
    });
}

/// Spawn a thread-safe [`Future`] on the *test* runtime.
///
/// See the [module documentation] for more information about the *test*
/// runtime.
///
/// [module documentation]: crate::test
pub fn spawn_future<Fut>(future: Fut, options: FutureOptions)
where
    Fut: Future<Output = ()> + Send + std::marker::Sync + 'static,
{
    run_on_test_runtime(move |mut runtime_ref| {
        runtime_ref.spawn_future(future, options);
        Ok(())
    });
}

/// Returned by [`join`] and [`join_many`].
#[derive(Copy, Clone, Debug)]
#[must_use = "this `JoinResult` should be handled"]
pub enum JoinResult {
    /// Actor(s) finished.
    Ok,
    /// Waiting for the actors timed out.
    TimedOut,
}

impl JoinResult {
    /// Unwrap the `JoinResult` expecting [`JoinResult::Ok`].
    #[track_caller]
    pub fn unwrap(self) {
        if let JoinResult::TimedOut = self {
            panic!("joining actors timed out");
        }
    }
}

/// Wait for the actor behind `actor_ref` to finish.
///
/// See [`join_many`] for more documentation.
pub fn join<M>(actor_ref: &ActorRef<M>, timeout: Duration) -> JoinResult {
    join_many(slice::from_ref(actor_ref), timeout)
}

/// Wait for all actors behind the `actor_refs` to finish.
///
/// # Notes
///
/// If you want to wait for actors with different message types try
/// [`ActorRef::map`] or [`ActorRef::try_map`].
pub fn join_many<M>(actor_refs: &[ActorRef<M>], timeout: Duration) -> JoinResult {
    let waker = SyncWaker::new();
    let start = Instant::now();
    for actor_ref in actor_refs {
        let elapsed = start.elapsed();
        if elapsed > timeout {
            return JoinResult::TimedOut;
        }
        match waker.clone().block_for(actor_ref.join(), timeout - elapsed) {
            Some(()) => {}
            None => return JoinResult::TimedOut,
        }
    }
    JoinResult::Ok
}

/// Wait for all `actors` in the group to finish.
///
/// # Notes
///
/// If you want to wait for actors with different message types try
/// [`ActorRef::map`] or [`ActorRef::try_map`].
pub fn join_all<M>(actors: &ActorGroup<M>, timeout: Duration) -> JoinResult {
    match SyncWaker::new().block_for(actors.join_all(), timeout) {
        Some(()) => JoinResult::Ok,
        None => JoinResult::TimedOut,
    }
}

/// Initialise a thread-local actor.
#[allow(clippy::type_complexity)]
pub fn init_local_actor<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<RuntimeAccess = ThreadLocal>,
{
    let (sender, receiver) = inbox::new_small();
    let ctx = actor::Context::new(receiver, ThreadLocal::new(runtime()));
    let actor = new_actor.new(ctx, arg)?;
    Ok((actor, ActorRef::local(sender)))
}

/// Initialise a thread-safe actor.
#[allow(clippy::type_complexity)]
pub fn init_actor<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<RuntimeAccess = ThreadSafe>,
{
    let (sender, receiver) = inbox::new_small();
    let ctx = actor::Context::new(receiver, ThreadSafe::new(shared_internals()));
    let actor = new_actor.new(ctx, arg)?;
    Ok((actor, ActorRef::local(sender)))
}

/// Spawn a synchronous actor.
///
/// This returns the thread handle for the thread the synchronous actor is
/// running on and an actor reference to the actor.
pub fn spawn_sync_actor<S, A, Arg, M>(
    supervisor: S,
    actor: A,
    arg: Arg,
    options: SyncActorOptions,
) -> io::Result<(thread::JoinHandle<()>, ActorRef<M>)>
where
    S: SyncSupervisor<A> + Send + 'static,
    A: SyncActor<Message = M, Argument = Arg, RuntimeAccess = Sync> + Send + 'static,
    Arg: Send + 'static,
    M: Send + 'static,
{
    static SYNC_WORKER_TEST_ID: AtomicUsize = AtomicUsize::new(10_000);
    let id = SYNC_WORKER_TEST_ID.fetch_add(1, Ordering::AcqRel);

    let shared = shared_internals();
    sync_worker::start(id, supervisor, actor, arg, options, shared, None).map(
        |(worker, actor_ref)| {
            let handle = worker.into_handle();
            (handle, actor_ref)
        },
    )
}

/// Poll a future.
///
/// # Notes
///
/// Wake notifications will be ignored, if this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_future<Fut>(future: Pin<&mut Fut>) -> Poll<Fut::Output>
where
    Fut: Future + ?Sized,
{
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(waker);
    Future::poll(future, &mut ctx)
}

/// Poll an [`AsyncIterator`].
///
/// # Notes
///
/// Wake notifications will be ignored, if this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_next<I>(iter: Pin<&mut I>) -> Poll<Option<I::Item>>
where
    I: AsyncIterator + ?Sized,
{
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(waker);
    AsyncIterator::poll_next(iter, &mut ctx)
}

/// Poll an actor.
///
/// This is effectively the same function as [`poll_future`], but instead polls
/// an actor.
///
/// # Notes
///
/// Wake notifications will be ignored, if this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_actor<A>(actor: Pin<&mut A>) -> Poll<Result<(), A::Error>>
where
    A: Actor + ?Sized,
{
    let waker = task::Waker::noop();
    let mut ctx = task::Context::from_waker(waker);
    Actor::try_poll(actor, &mut ctx)
}

/// Assert that a `Future` is not moved between calls.
#[cfg(test)]
pub(crate) struct AssertUnmoved<Fut> {
    future: Fut,
    /// Last place the future was polled, or null if never pulled.
    last_place: *const Self,
}

#[cfg(test)]
impl<Fut> AssertUnmoved<Fut> {
    /// Create a new `AssertUnmoved`.
    pub(crate) const fn new(future: Fut) -> AssertUnmoved<Fut> {
        AssertUnmoved {
            future,
            last_place: std::ptr::null(),
        }
    }
}

#[cfg(test)]
impl<Fut> Future for AssertUnmoved<Fut>
where
    Fut: Future,
{
    type Output = Fut::Output;

    #[track_caller]
    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let place = std::ptr::from_ref(&*self);
        if self.last_place.is_null() {
            unsafe { Pin::map_unchecked_mut(self.as_mut(), |s| &mut s.last_place).set(place) }
        } else {
            assert_eq!(
                self.last_place, place,
                "AssertUnmoved moved between poll calls"
            );
        }
        unsafe { Pin::map_unchecked_mut(self, |s| &mut s.future).poll(ctx) }
    }
}

#[cfg(test)]
unsafe impl<Fut: Send> Send for AssertUnmoved<Fut> {}
#[cfg(test)]
unsafe impl<Fut: std::marker::Sync> std::marker::Sync for AssertUnmoved<Fut> {}

#[cfg(test)]
pub(crate) struct TestAssertUnmovedNewActor<RT>(std::marker::PhantomData<RT>);

#[cfg(test)]
impl<RT> TestAssertUnmovedNewActor<RT> {
    pub(crate) const fn new() -> TestAssertUnmovedNewActor<RT> {
        TestAssertUnmovedNewActor(std::marker::PhantomData)
    }
}

#[cfg(test)]
impl<RT> NewActor for TestAssertUnmovedNewActor<RT> {
    type Message = ();
    type Argument = ();
    type Actor = AssertUnmoved<std::future::Pending<Result<(), !>>>;
    type Error = !;
    type RuntimeAccess = RT;

    fn new(
        &mut self,
        _: actor::Context<Self::Message, Self::RuntimeAccess>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        Ok(AssertUnmoved::new(std::future::pending()))
    }
}

#[cfg(test)]
#[track_caller]
pub(crate) fn assert_size<T>(expected: usize) {
    assert_eq!(size_of::<T>(), expected);
}

/// Supervisor to use in testing.
///
/// # Examples
///
/// Actor returning an error
///
/// ```should_panic
/// use std::io;
///
/// use heph::{actor, actor_fn};
/// use heph_rt::ThreadLocal;
/// use heph_rt::spawn::options::ActorOptions;
/// use heph_rt::test::{TestSupervisor, spawn_local};
///
/// // Create a new supervisor.
/// // NOTE: we don't need to do anything witht the checker, just let it drop
/// // out of scope and it will check the error of the spawned actor.
/// let (supervisor, _checker) = TestSupervisor::new();
///
/// async fn error_actor(_: actor::Context<(), ThreadLocal>) -> Result<(), io::Error>{
///   Err(io::Error::new(io::ErrorKind::Other, "oops!"))
/// }
/// spawn_local(supervisor, actor_fn(error_actor), (), ActorOptions::default());
///
/// // checker will drop out of scope here and panic due to the error in the
/// // spawned actor.
/// ```
///
/// Actor panicking.
///
/// ```should_panic
/// use heph::{actor, actor_fn};
/// use heph_rt::ThreadLocal;
/// use heph_rt::spawn::options::ActorOptions;
/// use heph_rt::test::{TestSupervisor, spawn_local};
///
/// // Create a new supervisor.
/// // NOTE: we don't need to do anything witht the checker, just let it drop
/// // out of scope and it will check the error of the spawned actor.
/// let (supervisor, _checker) = TestSupervisor::new();
///
/// async fn panic_actor(_: actor::Context<(), ThreadLocal>) {
///   panic!("oops!")
/// }
/// spawn_local(supervisor, actor_fn(panic_actor), (), ActorOptions::default());
///
/// // checker will drop out of scope here and panic due to the panic in the
/// // spawned actor.
/// ```
///
/// Actor running without issues.
///
/// ```
/// # #![feature(never_type)]
/// use heph::{actor, actor_fn};
/// use heph_rt::ThreadLocal;
/// use heph_rt::spawn::options::ActorOptions;
/// use heph_rt::test::{TestSupervisor, spawn_local};
///
/// // Create a new supervisor.
/// // NOTE: we don't need to do anything witht the checker, just let it drop
/// // out of scope and it will check the error of the spawned actor.
/// let (supervisor, _checker) = TestSupervisor::new();
///
/// async fn ok_actor(_: actor::Context<(), ThreadLocal>) -> Result<(), !> {
///   Ok(())
/// }
/// spawn_local(supervisor, actor_fn(ok_actor), (), ActorOptions::default());
///
/// // checker will drop out of scope here do nothing as the actor ran without
/// // issues.
/// ```
#[derive(Debug)]
pub struct TestSupervisor<E> {
    result: Option<oneshot::Sender<TestResult<E>>>,
}

/// Result used in [`TestSupervisor`].
#[derive(Debug)]
enum TestResult<E> {
    Error(E),
    Panic(Box<dyn Any + Send + 'static>),
}

impl<E: fmt::Display> TestSupervisor<E> {
    /// Create a new `TestSupervisor` and accompanying `ResultCheck`
    pub fn new() -> (TestSupervisor<E>, ResultCheck<E>) {
        let (sender, receiver) = new_oneshot();
        let supervisor = TestSupervisor {
            result: Some(sender),
        };
        let checker = ResultCheck { result: receiver };
        (supervisor, checker)
    }
}

impl<NA> Supervisor<NA> for TestSupervisor<<NA::Actor as Actor>::Error>
where
    NA: NewActor,
{
    fn decide(&mut self, err: <NA::Actor as Actor>::Error) -> SupervisorStrategy<NA::Argument> {
        _ = self.result.take().unwrap().try_send(TestResult::Error(err));
        SupervisorStrategy::Stop
    }

    fn decide_on_restart_error(&mut self, _: NA::Error) -> SupervisorStrategy<NA::Argument> {
        panic!("Don't call TestSupervisor::decide_on_restart_error");
    }

    fn second_restart_error(&mut self, _: NA::Error) {
        panic!("Don't call TestSupervisor::second_restart_error");
    }

    fn decide_on_panic(
        &mut self,
        panic: Box<dyn Any + Send + 'static>,
    ) -> SupervisorStrategy<NA::Argument> {
        _ = self
            .result
            .take()
            .unwrap()
            .try_send(TestResult::Panic(panic));
        SupervisorStrategy::Stop
    }
}

impl<A> SyncSupervisor<A> for TestSupervisor<A::Error>
where
    A: SyncActor,
{
    fn decide(&mut self, err: A::Error) -> SupervisorStrategy<A::Argument> {
        _ = self.result.take().unwrap().try_send(TestResult::Error(err));
        SupervisorStrategy::Stop
    }

    fn decide_on_panic(
        &mut self,
        panic: Box<dyn Any + Send + 'static>,
    ) -> SupervisorStrategy<A::Argument> {
        _ = self
            .result
            .take()
            .unwrap()
            .try_send(TestResult::Panic(panic));
        SupervisorStrategy::Stop
    }
}

/// Just let it drop.
///
/// See [`TestSupervisor`] for usage.
#[derive(Debug)]
pub struct ResultCheck<E: fmt::Display> {
    result: oneshot::Receiver<TestResult<E>>,
}

impl<E: fmt::Display> Drop for ResultCheck<E> {
    fn drop(&mut self) {
        // NOTE: don't want to panic while panicking.
        if !thread::panicking() {
            let waker = SyncWaker::new();
            match waker.block_on(self.result.recv()) {
                Some(TestResult::Error(err)) => panic!("unexpected error: {err}"),
                Some(TestResult::Panic(panic)) => resume_unwind(panic),
                None => { /* Actor ran without issues. */ }
            }
        }
    }
}
