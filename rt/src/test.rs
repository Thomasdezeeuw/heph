//! Testing facilities.
//!
//! This module will lazily create an single-threaded *test* runtime. Functions
//! such as [`try_spawn_local`] will use this to spawn and run actors. The
//! *test* runtime will not stop and the thread's resources are not cleaned up
//! (properly).
//!
//! Available utilities:
//!  * Blocking on [`Future`]s:
//!    * [`block_on`]: spawns a `Future` and waits for the result.
//!  * Spawning:
//!    * [`try_spawn_local`]: attempt to spawn a thread-local [actor].
//!    * [`try_spawn`]: attempt to spawn a thread-safe [actor].
//!    * [`spawn_sync_actor`]: spawn a [synchronous actor].
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
//!    * [`poll_next`]: poll a [`AsyncIterator`].
//!  * Miscellaneous:
//!    * [`size_of_actor`], [`size_of_actor_val`]: returns the size of an actor.
//!    * [`set_message_loss`]: set the percentage of messages lost on purpose.
//!    * [`PanicSupervisor`]: supervisor that panics when it receives an actor's
//!      error.
//!
//! [actor]: actor
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
//! [dev-dependencies.heph]
//! features = ["test"]
//! ```

use std::async_iter::AsyncIterator;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;
use std::task::{self, Poll};
use std::time::{Duration, Instant};
use std::{io, slice, thread};

use heph::actor::{self, Actor, NewActor, SyncActor, SyncWaker};
use heph::actor_ref::{ActorGroup, ActorRef};
use heph::supervisor::{Supervisor, SyncSupervisor};
use heph_inbox::oneshot::new_oneshot;
use heph_inbox::Manager;

use crate::shared::waker;
use crate::spawn::{ActorOptions, FutureOptions, SyncActorOptions};
use crate::sync_worker::SyncWorker;
use crate::thread_waker::ThreadWaker;
use crate::worker::{Control, Worker};
use crate::{
    self as rt, shared, ProcessId, RuntimeRef, Sync, ThreadLocal, ThreadSafe, SYNC_WORKER_ID_END,
    SYNC_WORKER_ID_START,
};

#[doc(no_inline)]
#[cfg(feature = "test")]
pub use heph::test::*;

pub(crate) const TEST_PID: ProcessId = ProcessId(0);

pub(crate) static NOOP_WAKER: LazyLock<ThreadWaker> = LazyLock::new(|| {
    let poll = mio::Poll::new().expect("failed to create `Poll` instance for test module");
    let waker = mio::Waker::new(poll.registry(), mio::Token(0))
        .expect("failed to create `Waker` instance for test module");
    ThreadWaker::new(waker)
});

static SHARED_INTERNAL: LazyLock<Arc<shared::RuntimeInternals>> = LazyLock::new(|| {
    let setup = shared::RuntimeInternals::setup()
        .expect("failed to setup runtime internals for test module");
    Arc::new_cyclic(|shared_internals| {
        let waker_id = waker::init(shared_internals.clone());
        let worker_wakers = vec![&*NOOP_WAKER].into_boxed_slice();
        setup.complete(waker_id, worker_wakers, None)
    })
});

/// Returns a reference to a fake local runtime.
///
/// # Notes
///
/// The returned runtime reference is **not** a reference to the *test* runtime
/// as described in the module documentation.
pub fn runtime() -> RuntimeRef {
    thread_local! {
        /// Per thread runtime.
        static TEST_RT: Worker = {
            let (_, receiver) = rt::channel::new()
                .expect("failed to create runtime channel for test module");
            Worker::new_test(SHARED_INTERNAL.clone(), receiver)
                .expect("failed to create local `Runtime` for test module")
        };
    }

    TEST_RT.with(Worker::create_ref)
}

/// Sending side of a runtime channel to control the test runtime.
type RtControl = rt::channel::Sender<Control>;

/// Lazily start the *test* runtime on a new thread, returning the control
/// channel.
fn test_runtime() -> &'static RtControl {
    static TEST_RT: LazyLock<RtControl> = LazyLock::new(|| {
        let (sender, receiver) =
            rt::channel::new().expect("failed to create runtime channel for test module");
        let _handle = thread::Builder::new()
            .name("Heph Test Runtime".to_string())
            .spawn(move || {
                // NOTE: because we didn't indicate the runtime has started this
                // will never stop.
                Worker::new_test(SHARED_INTERNAL.clone(), receiver)
                    .expect("failed to create a runtime for test module")
                    .run()
                    .expect("failed to run test runtime");
            })
            .expect("failed to start thread for test runtime");
        sender
    });

    &TEST_RT
}

/// Run function `f` on the *test* runtime.
fn run_on_test_runtime<F>(f: F)
where
    F: FnOnce(RuntimeRef) -> Result<(), String> + Send + 'static,
{
    test_runtime()
        .try_send(Control::Run(Box::new(f)))
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
    let _ = receiver.register_waker(&task::Waker::from(waker.clone()));
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
pub fn block_on<Fut>(future: Fut) -> Fut::Output
where
    Fut: Future + Send + 'static,
    Fut::Output: Send,
{
    let (sender, receiver) = new_oneshot();
    let waker = SyncWaker::new();
    spawn_local_future(
        async move {
            let result = future.await;
            assert!(
                sender.try_send(result).is_ok(),
                "failed to return future result"
            );
        },
        FutureOptions::default(),
    );
    waker
        .block_on(receiver.recv_once())
        .expect("failed to receive result from future")
}

/// Attempt to spawn a thread-local actor on the *test* runtime.
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

/// Attempt to spawn a thread-safe actor on the *test* runtime.
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
    NA: NewActor<RuntimeAccess = ThreadSafe> + std::marker::Sync + Send + 'static,
    NA::Actor: Send + std::marker::Sync + 'static,
    NA::Message: Send,
    NA::Argument: Send,
    NA::Error: Send,
{
    run_on_test_runtime_wait(move |mut runtime_ref| {
        runtime_ref.try_spawn(supervisor, new_actor, arg, options)
    })
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
    new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<RuntimeAccess = ThreadLocal>,
{
    init_local_actor_with_inbox(new_actor, arg).map(|(actor, _, actor_ref)| (actor, actor_ref))
}

/// Initialise a thread-safe actor.
#[allow(clippy::type_complexity)]
pub fn init_actor<NA>(
    new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<RuntimeAccess = ThreadSafe>,
{
    init_actor_with_inbox(new_actor, arg).map(|(actor, _, actor_ref)| (actor, actor_ref))
}

/// Initialise a thread-local actor with access to it's inbox.
#[allow(clippy::type_complexity)]
pub(crate) fn init_local_actor_with_inbox<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, Manager<NA::Message>, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<RuntimeAccess = ThreadLocal>,
{
    let (manager, sender, receiver) = Manager::new_small_channel();
    let ctx = actor::Context::new(receiver, ThreadLocal::new(TEST_PID, runtime()));
    let actor = new_actor.new(ctx, arg)?;
    Ok((actor, manager, ActorRef::local(sender)))
}

/// Initialise a thread-safe actor with access to it's inbox.
#[allow(clippy::type_complexity)]
pub(crate) fn init_actor_with_inbox<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, Manager<NA::Message>, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<RuntimeAccess = ThreadSafe>,
{
    let (manager, sender, receiver) = Manager::new_small_channel();
    let ctx = actor::Context::new(receiver, ThreadSafe::new(TEST_PID, SHARED_INTERNAL.clone()));
    let actor = new_actor.new(ctx, arg)?;
    Ok((actor, manager, ActorRef::local(sender)))
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
    static SYNC_WORKER_TEST_ID: AtomicUsize = AtomicUsize::new(SYNC_WORKER_ID_START);
    let id = SYNC_WORKER_TEST_ID.fetch_add(1, Ordering::SeqCst);
    assert!(
        id < SYNC_WORKER_ID_END,
        "spawned too many synchronous test actors"
    );

    let shared = runtime().clone_shared();
    SyncWorker::start(id, supervisor, actor, arg, options, shared, None).map(
        |(worker, actor_ref)| {
            let handle = worker.into_handle();
            (handle, actor_ref)
        },
    )
}

/// Poll a future.
///
/// The [`task::Context`] will be provided by the *test* runtime.
///
/// # Notes
///
/// Wake notifications will be ignored, if this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_future<Fut>(future: Pin<&mut Fut>) -> Poll<Fut::Output>
where
    Fut: Future + ?Sized,
{
    let waker = runtime().new_local_task_waker(TEST_PID);
    let mut ctx = task::Context::from_waker(&waker);
    Future::poll(future, &mut ctx)
}

/// Poll a [`AsyncIterator`].
///
/// The [`task::Context`] will be provided by the *test* runtime.
///
/// # Notes
///
/// Wake notifications will be ignored, if this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_next<I>(iter: Pin<&mut I>) -> Poll<Option<I::Item>>
where
    I: AsyncIterator + ?Sized,
{
    let waker = runtime().new_local_task_waker(TEST_PID);
    let mut ctx = task::Context::from_waker(&waker);
    AsyncIterator::poll_next(iter, &mut ctx)
}

/// Poll an actor.
///
/// This is effectively the same function as [`poll_future`], but instead polls
/// an actor. The [`task::Context`] will be provided by the *test* runtime.
///
/// # Notes
///
/// Wake notifications will be ignored, if this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_actor<A>(actor: Pin<&mut A>) -> Poll<Result<(), A::Error>>
where
    A: Actor + ?Sized,
{
    let waker = runtime().new_local_task_waker(TEST_PID);
    let mut ctx = task::Context::from_waker(&waker);
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
        let place = &*self as *const Self;
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
