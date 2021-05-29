//! Testing facilities.
//!
//! This module will lazily create an single-threaded *test* runtime. Functions
//! such as [`try_spawn_local`] will use this to spawn and run actors. The
//! *test* runtime will not stop and the thread's resources are not cleaned up
//! (properly).
//!
//! Available utilities:
//!  * Spawning:
//!    * [`try_spawn_local`]: attempt to spawn a thread-local [actor].
//!    * [`try_spawn`]: attempt to spawn a thread-safe [actor].
//!    * [`spawn_future`]: spawn a thread-local [`Future`].
//!    * [`spawn_sync_actor`]: spawn a [synchronous actor].
//!  * Initialising actors:
//!    * [`init_local_actor`]: initialise a thread-local actor.
//!    * [`init_actor`]: initialise a thread-safe actor.
//!  * Polling:
//!    * [`poll_actor`]: poll an [`Actor`].
//!    * [`poll_future`]: poll a [`Future`].
//!    * [`poll_next`]: poll a [`Stream`].
//!  * Miscellaneous:
//!    * [`size_of_actor`], [`size_of_actor_val`]: returns the size of an actor.
//!    * [`set_message_loss`]: set the percentage of messages lost on purpose.
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

use std::future::Future;
use std::lazy::SyncLazy;
use std::mem::size_of;
use std::pin::Pin;
use std::stream::Stream;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{self, Poll};
use std::{io, slice, thread};

use getrandom::getrandom;
use heph_inbox::oneshot::new_oneshot;
use heph_inbox::Manager;
use log::warn;

use crate::actor::{self, Actor, NewActor, SyncActor, SyncWaker};
use crate::actor_ref::ActorRef;
use crate::rt::local::{Control, Runtime};
use crate::rt::shared::waker;
use crate::rt::sync_worker::SyncWorker;
use crate::rt::thread_waker::ThreadWaker;
use crate::rt::{
    self, shared, ProcessId, RuntimeRef, ThreadLocal, ThreadSafe, SYNC_WORKER_ID_END,
    SYNC_WORKER_ID_START,
};
use crate::spawn::{ActorOptions, FutureOptions, SyncActorOptions};
use crate::supervisor::{Supervisor, SyncSupervisor};

pub(crate) const TEST_PID: ProcessId = ProcessId(0);

pub(crate) static NOOP_WAKER: SyncLazy<ThreadWaker> = SyncLazy::new(|| {
    let poll = mio::Poll::new().expect("failed to create `Poll` instance for test module");
    let waker = mio::Waker::new(poll.registry(), mio::Token(0))
        .expect("failed to create `Waker` instance for test module");
    ThreadWaker::new(waker)
});

static SHARED_INTERNAL: SyncLazy<Arc<shared::RuntimeInternals>> = SyncLazy::new(|| {
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
        static TEST_RT: Runtime = {
            let (_, receiver) = rt::channel::new()
                .expect("failed to create runtime channel for test module");
            Runtime::new_test(SHARED_INTERNAL.clone(), receiver)
                .expect("failed to create local `Runtime` for test module")
        };
    }

    TEST_RT.with(Runtime::create_ref)
}

/// Sending side of a runtime channel to control the test runtime.
type RtControl = rt::channel::Sender<Control>;

/// Lazily start the *test* runtime on a new thread, returning the control
/// channel.
fn test_runtime() -> &'static RtControl {
    static TEST_RT: SyncLazy<RtControl> = SyncLazy::new(|| {
        let (sender, receiver) =
            rt::channel::new().expect("failed to create runtime channel for test module");
        let _handle = thread::Builder::new()
            .name("Heph Test Runtime".to_string())
            .spawn(move || {
                // NOTE: because we didn't indicate the runtime has started this
                // will never stop.
                Runtime::new_test(SHARED_INTERNAL.clone(), receiver)
                    .expect("failed to create a runtime for test module")
                    .run_event_loop()
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
        .block_on(receiver.recv())
        .expect("failed to receive result from test runtime")
        .0
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
    S: Supervisor<NA> + Send + Sync + 'static,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Sync + Send + 'static,
    NA::Actor: Send + Sync + 'static,
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
pub fn spawn_future<Fut>(future: Fut, options: FutureOptions)
where
    Fut: Future<Output = ()> + Send + Sync + 'static,
{
    run_on_test_runtime(move |mut runtime_ref| {
        runtime_ref.spawn_future(future, options);
        Ok(())
    });
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
pub fn spawn_sync_actor<S, A, E, Arg, M>(
    supervisor: S,
    actor: A,
    arg: Arg,
    options: SyncActorOptions,
) -> io::Result<(thread::JoinHandle<()>, ActorRef<M>)>
where
    S: SyncSupervisor<A> + Send + 'static,
    A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
    Arg: Send + 'static,
    M: Send + 'static,
{
    static SYNC_WORKER_TEST_ID: AtomicUsize = AtomicUsize::new(SYNC_WORKER_ID_START);
    let id = SYNC_WORKER_TEST_ID.fetch_add(1, Ordering::SeqCst);
    if id >= SYNC_WORKER_ID_END {
        panic!("spawned too many synchronous test actors");
    }

    SyncWorker::start(id, supervisor, actor, arg, options, None).map(|(worker, actor_ref)| {
        let handle = worker.into_handle();
        (handle, actor_ref)
    })
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

/// Poll a stream.
///
/// The [`task::Context`] will be provided by the *test* runtime.
///
/// # Notes
///
/// Wake notifications will be ignored, if this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_next<S>(stream: Pin<&mut S>) -> Poll<Option<S::Item>>
where
    S: Stream + ?Sized,
{
    let waker = runtime().new_local_task_waker(TEST_PID);
    let mut ctx = task::Context::from_waker(&waker);
    Stream::poll_next(stream, &mut ctx)
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

/// Percentage of messages lost on purpose.
static MSG_LOSS: AtomicU8 = AtomicU8::new(0);

/// Set the percentage of messages lost on purpose.
///
/// This is useful to test the resilience of actors with respect to message
/// loss. Any and all messages send, thus including remote and local messages,
/// could be lost on purpose when using this function.
///
/// Note that the sending of the messages will not return an error if the
/// message is lost using this function.
///
/// `percent` must be number between `0` and `100`, setting this to `0` (the
/// default) will disable the message loss.
pub fn set_message_loss(mut percent: u8) {
    if percent > 100 {
        percent = 100;
    }
    MSG_LOSS.store(percent, Ordering::SeqCst)
}

/// Returns `true` if the message should be lost.
pub(crate) fn should_lose_msg() -> bool {
    // Safety: `Relaxed` is fine here as we'll get the update, sending a message
    // when we're not supposed to isn't too bad.
    let loss = MSG_LOSS.load(Ordering::Relaxed);
    loss != 0 || random_percentage() < loss
}

/// Returns a number between [0, 100].
fn random_percentage() -> u8 {
    let mut p = 0;
    if let Err(err) = getrandom(slice::from_mut(&mut p)) {
        warn!("error getting random bytes: {}", err);
        100
    } else {
        p % 100
    }
}

/// Returns the size of the actor.
///
/// When using asynchronous function for actors see [`size_of_actor_val`].
pub const fn size_of_actor<NA>() -> usize
where
    NA: NewActor,
{
    size_of::<NA::Actor>()
}

/// Returns the size of the point-to actor.
///
/// # Examples
///
/// ```
/// use heph::actor;
/// use heph::test::size_of_actor_val;
/// use heph::rt::ThreadLocal;
///
/// async fn actor(mut ctx: actor::Context<String, ThreadLocal>) {
///     // Receive a message.
///     if let Ok(msg) = ctx.receive_next().await {
///         // Print the message.
///         println!("got a message: {}", msg);
///     }
/// }
///
/// assert_eq!(size_of_actor_val(&(actor as fn(_) -> _)), 64);
/// ```
pub const fn size_of_actor_val<NA>(_: &NA) -> usize
where
    NA: NewActor,
{
    size_of_actor::<NA>()
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
unsafe impl<Fut: Sync> Sync for AssertUnmoved<Fut> {}
