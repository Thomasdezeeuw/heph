//! Testing facilities.
//!
//! This module will lazily create an active, but not running, runtime per
//! thread.
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

use std::cell::RefCell;
use std::cmp::max;
use std::future::Future;
use std::lazy::SyncLazy;
use std::mem::size_of;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{self, Poll};
use std::{io, thread};

use inbox::Manager;
use rand::Rng;

use crate::actor::sync::SyncActor;
use crate::actor::{self, context, Actor, NewActor};
use crate::actor_ref::ActorRef;
use crate::rt::scheduler::Scheduler;
use crate::rt::sync_worker::SyncWorker;
use crate::rt::waker::{self, WakerId};
use crate::rt::worker::RunningRuntime;
use crate::rt::{
    self, ProcessId, RuntimeRef, SharedRuntimeInternal, SyncActorOptions, Timers,
    SYNC_WORKER_ID_END, SYNC_WORKER_ID_START,
};
use crate::supervisor::SyncSupervisor;

pub(crate) const TEST_PID: ProcessId = ProcessId(0);

static POLL: SyncLazy<mio::Poll> =
    SyncLazy::new(|| mio::Poll::new().expect("failed to create `Poll` instance for test module"));

static COORDINATOR_ID: SyncLazy<WakerId> = SyncLazy::new(|| {
    let waker = mio::Waker::new(POLL.registry(), mio::Token(0)).unwrap();
    let (sender, _) = crossbeam_channel::unbounded();
    waker::init(waker, sender)
});

static SHARED_INTERNAL: SyncLazy<Arc<SharedRuntimeInternal>> = SyncLazy::new(|| {
    let registry = POLL
        .registry()
        .try_clone()
        .expect("failed to clone `Registry` for test module");
    let scheduler = Scheduler::new();
    let scheduler = scheduler.create_ref();
    let timers = Arc::new(Mutex::new(Timers::new()));
    SharedRuntimeInternal::new(*COORDINATOR_ID, scheduler, registry, timers)
});

thread_local! {
    /// Per thread active, but not running, runtime.
    static TEST_RT: RefCell<RunningRuntime> = {
        // NOTE: `sender` needs to live during `RunningRuntime::init`.
        let (_sender, receiver) = rt::channel::new().expect("failed to create Channel for test module");
        RefCell::new(RunningRuntime::init(receiver, SHARED_INTERNAL.clone(), None).expect("failed to create local Runtime for test module"))
    };
}

/// Returns a reference to the *test* runtime.
pub fn runtime() -> RuntimeRef {
    TEST_RT.with(|runtime| runtime.borrow().create_ref())
}

/// Initialise a thread-local actor.
#[allow(clippy::type_complexity)]
pub fn init_local_actor<NA>(
    new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<Context = context::ThreadLocal>,
{
    init_local_actor_with_inbox(new_actor, arg).map(|(actor, _, actor_ref)| (actor, actor_ref))
}

/// Initialise a thread-local actor with access to it's inbox.
#[allow(clippy::type_complexity)]
pub(crate) fn init_local_actor_with_inbox<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, Manager<NA::Message>, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<Context = context::ThreadLocal>,
{
    let (manager, sender, receiver) = Manager::new_small_channel();
    let ctx = actor::Context::new_local(TEST_PID, receiver, runtime());
    let actor = new_actor.new(ctx, arg)?;
    Ok((actor, manager, ActorRef::local(sender)))
}

/// Initialise a thread-safe actor with access to it's inbox.
#[allow(clippy::type_complexity)]
#[cfg(test)]
pub(crate) fn init_actor_with_inbox<NA>(
    mut new_actor: NA,
    arg: NA::Argument,
) -> Result<(NA::Actor, Manager<NA::Message>, ActorRef<NA::Message>), NA::Error>
where
    NA: NewActor<Context = context::ThreadSafe>,
{
    let (manager, sender, receiver) = Manager::new_small_channel();
    let ctx = actor::Context::new_shared(TEST_PID, receiver, SHARED_INTERNAL.clone());
    let actor = new_actor.new(ctx, arg)?;
    Ok((actor, manager, ActorRef::local(sender)))
}

/// Spawn a synchronous actor.
///
/// This returns the thread handle for the thread the synchronous actor is
/// running on and an actor reference to the actor.
pub fn spawn_sync_actor<Sv, A, E, Arg, M>(
    supervisor: Sv,
    actor: A,
    arg: Arg,
    options: SyncActorOptions,
) -> io::Result<(thread::JoinHandle<()>, ActorRef<M>)>
where
    Sv: SyncSupervisor<A> + Send + 'static,
    A: SyncActor<Message = M, Argument = Arg, Error = E> + Send + 'static,
    Arg: Send + 'static,
    M: Send + 'static,
{
    static SYNC_WORKER_TEST_ID: AtomicUsize = AtomicUsize::new(SYNC_WORKER_ID_START);
    let id = SYNC_WORKER_TEST_ID.fetch_add(1, Ordering::SeqCst);
    if id >= SYNC_WORKER_ID_END {
        panic!("spawned too many synchronous test actors");
    }

    SyncWorker::start(id, supervisor, actor, arg, options).map(|(worker, actor_ref)| {
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
/// Wake notifications will be ignored. If this is required run an end to end
/// test with a completely functional runtime instead.
pub fn poll_future<Fut>(future: Pin<&mut Fut>) -> Poll<Fut::Output>
where
    Fut: Future,
{
    let waker = runtime().new_local_task_waker(TEST_PID);
    let mut ctx = task::Context::from_waker(&waker);
    Future::poll(future, &mut ctx)
}

/// Poll an actor.
///
/// This is effectively the same function as [`poll_future`], but instead polls
/// an actor. The [`task::Context`] will be provided by the *test* runtime.
///
/// # Notes
///
/// Wake notifications will be ignored. If this is required run an end to end
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
pub fn set_message_loss(percent: u8) {
    let percent = max(percent, 100);
    MSG_LOSS.store(percent, Ordering::Release)
}

/// Returns `true` if the message should be lost.
pub(crate) fn should_lose_msg() -> bool {
    let loss = MSG_LOSS.load(Ordering::Relaxed);
    loss != 0 && rand::thread_rng().gen_range(0, 100) < loss
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
/// # #![feature(never_type)]
/// #
/// use heph::actor;
/// use heph::test::size_of_actor_val;
///
/// async fn actor(mut ctx: actor::Context<String>) -> Result<(), !> {
///     // Receive a message.
///     if let Ok(msg) = ctx.receive_next().await {
///         // Print the message.
///         println!("got a message: {}", msg);
///     }
///     // And we're done.
///     Ok(())
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

#[test]
fn test_size_of_actor() {
    use crate::actor::context::ThreadLocal;

    async fn actor1(_: actor::Context<!>) -> Result<(), !> {
        Ok(())
    }

    #[allow(trivial_casts)]
    {
        assert_eq!(size_of_actor_val(&(actor1 as fn(_) -> _)), 32);
    }

    struct NA;

    impl NewActor for NA {
        type Message = !;
        type Argument = ();
        type Actor = A;
        type Error = !;
        type Context = ThreadLocal;

        fn new(
            &mut self,
            _: actor::Context<Self::Message, Self::Context>,
            _: Self::Argument,
        ) -> Result<Self::Actor, Self::Error> {
            Ok(A)
        }
    }

    struct A;

    impl Actor for A {
        type Error = !;
        fn try_poll(
            self: Pin<&mut Self>,
            _: &mut task::Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    assert_eq!(size_of::<A>(), 0);
    assert_eq!(size_of_actor::<NA>(), 0);
    assert_eq!(size_of_actor_val(&NA), 0);
}
