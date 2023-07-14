//! Module containing the types for synchronous actors.

use std::future::Future;
use std::io;
use std::pin::pin;
use std::task::{self, Poll, RawWaker, RawWakerVTable};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

use heph_inbox::Receiver;
use heph_inbox::{self as inbox, ReceiverConnected};
use log::trace;

use crate::actor::private::ActorResult;
use crate::actor::{ActorFn, NoMessages, RecvError};
use crate::actor_ref::ActorRef;
use crate::supervisor::{SupervisorStrategy, SyncSupervisor};

/// Synchronous actor.
///
/// Synchronous actors run on their own thread and therefore can perform
/// synchronous operations such as blocking I/O or time consuming computations.
/// Much like asynchronous [actors] the actor will be supplied with a [context],
/// which can be used for receiving messages. As with asynchronous actors
/// communication is done via message sending, using [actor references].
///
/// The easiest way to implement this trait by using functions, see the [module
/// level] documentation for an example of this. All functions *pointers* that
/// accept a [`SyncContext`] as argument and return `Result<(), Error>` or `()`
/// implement the `SyncActor` trait. There is also the [`ActorFn`] helper type
/// to implement the trait for any function.
///
/// [module level]: crate::actor
///
/// Synchronous actor can be started using [`spawn_sync_actor`].
///
/// # Panics
///
/// Panics are not caught and will **not** be returned to the actor's
/// supervisor.
///
/// [actors]: crate::Actor
/// [context]: SyncContext
/// [actor references]: crate::ActorRef
pub trait SyncActor {
    /// The type of messages the synchronous actor can receive.
    ///
    /// Using an enum allows an actor to handle multiple types of messages. See
    /// [`NewActor::Message`] for examples.
    ///
    /// [`NewActor::Message`]: crate::NewActor::Message
    type Message;

    /// The argument(s) passed to the actor.
    ///
    /// This works just like the [arguments in `NewActor`].
    ///
    /// [arguments in `NewActor`]: crate::NewActor::Argument
    type Argument;

    /// An error the actor can return to its [supervisor]. This error will be
    /// considered terminal for this actor and should **not** be an error of
    /// regular processing of a message.
    ///
    /// How to process non-terminal errors that happen during regular processing
    /// is up to the actor.
    ///
    /// [supervisor]: crate::supervisor::SyncSupervisor
    type Error;

    /// The kind of runtime access needed by the actor.
    ///
    /// The runtime is accessible via the actor's context. See
    /// [`SyncContext`] for more information.
    type RuntimeAccess;

    /// Run the synchronous actor.
    fn run(
        &self,
        ctx: SyncContext<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<(), Self::Error>;
}

/// Macro to implement the [`SyncActor`] trait on function pointers.
macro_rules! impl_sync_actor {
    (
        $( ( $( $arg_name: ident : $arg: ident ),* ) ),*
        $(,)?
    ) => {
        $(
            impl<M, RT, $( $arg, )* R> SyncActor for fn(ctx: SyncContext<M, RT>, $( $arg_name: $arg ),*) -> R
            where
                R: ActorResult,
            {
                type Message = M;
                type Argument = ($( $arg ),*);
                type Error = R::Error;
                type RuntimeAccess = RT;

                #[allow(non_snake_case)]
                fn run(&self, ctx: SyncContext<Self::Message, Self::RuntimeAccess>, arg: Self::Argument) -> Result<(), Self::Error> {
                    let ($( $arg ),*) = arg;
                    (self)(ctx, $( $arg ),*).into()
                }
            }

            impl<F, M, RT, $( $arg, )* R> SyncActor for ActorFn<F, M, RT, ($( $arg, )*), R>
            where
                F: Fn(SyncContext<M, RT>, $( $arg ),*) -> R,
                R: ActorResult,
            {
                type Message = M;
                type Argument = ($( $arg ),*);
                type Error = R::Error;
                type RuntimeAccess = RT;

                #[allow(non_snake_case)]
                fn run(&self, ctx: SyncContext<Self::Message, Self::RuntimeAccess>, arg: Self::Argument) -> Result<(), Self::Error> {
                    let ($( $arg ),*) = arg;
                    (self.inner)(ctx, $( $arg ),*).into()
                }
            }
        )*
    };
}

impl_sync_actor!(());

impl<M, RT, Arg, R> SyncActor for fn(ctx: SyncContext<M, RT>, arg: Arg) -> R
where
    R: ActorResult,
{
    type Message = M;
    type Argument = Arg;
    type Error = R::Error;
    type RuntimeAccess = RT;

    fn run(
        &self,
        ctx: SyncContext<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<(), Self::Error> {
        (self)(ctx, arg).into()
    }
}

impl<F, M, RT, Arg, R> SyncActor for ActorFn<F, M, RT, (Arg,), R>
where
    F: Fn(SyncContext<M, RT>, Arg) -> R,
    R: ActorResult,
{
    type Message = M;
    type Argument = Arg;
    type Error = R::Error;
    type RuntimeAccess = RT;

    #[allow(non_snake_case)]
    fn run(
        &self,
        ctx: SyncContext<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<(), Self::Error> {
        ((self.inner)(ctx, arg)).into()
    }
}

impl_sync_actor!(
    // NOTE: we don't want a single argument into tuple form so we implement
    // that manually above.
    (arg1: Arg1, arg2: Arg2),
    (arg1: Arg1, arg2: Arg2, arg3: Arg3),
    (arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4),
    (arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4, arg5: Arg5),
);

/// The context in which a synchronous actor is executed.
///
/// This context can be used for a number of things including receiving messages
/// and getting access to the runtime which is running the actor (`RT`).
///
/// Also see the asynchronous version: [`actor::Context`].
///
/// [`actor::Context`]: crate::actor::Context
#[derive(Debug)]
pub struct SyncContext<M, RT = ()> {
    inbox: Receiver<M>,
    future_waker: Option<SyncWaker>,
    /// Runtime access.
    rt: RT,
}

impl<M, RT> SyncContext<M, RT> {
    /// Create a new `SyncContext`.
    #[doc(hidden)] // Not part of the stable API.
    pub const fn new(inbox: Receiver<M>, rt: RT) -> SyncContext<M, RT> {
        SyncContext {
            inbox,
            future_waker: None,
            rt,
        }
    }

    /// Attempt to receive the next message.
    ///
    /// This will attempt to receive the next message if one is available. If
    /// the actor wants to wait until a message is received [`receive_next`] can
    /// be used, which blocks until a message is ready.
    ///
    /// [`receive_next`]: SyncContext::receive_next
    ///
    /// # Examples
    ///
    /// A synchronous actor that receives a name to greet, or greets the entire
    /// world.
    ///
    /// ```
    /// use heph::actor::SyncContext;
    ///
    /// fn greeter_actor(mut ctx: SyncContext<String>) {
    ///     if let Ok(name) = ctx.try_receive_next() {
    ///         println!("Hello {name}");
    ///     } else {
    ///         println!("Hello world");
    ///     }
    /// }
    ///
    /// # fn assert_sync_actor<A: heph::actor::SyncActor<RuntimeAccess = ()>>(_: A) { }
    /// # assert_sync_actor(greeter_actor as fn(_) -> _);
    /// ```
    pub fn try_receive_next(&mut self) -> Result<M, RecvError> {
        self.inbox.try_recv().map_err(RecvError::from)
    }

    /// Receive the next message.
    ///
    /// Returns the next message available. If no messages are currently
    /// available it will block until a message becomes available or until all
    /// actor references (that reference this actor) are dropped.
    ///
    /// # Examples
    ///
    /// An actor that waits for a message and prints it.
    ///
    /// ```
    /// use heph::actor::SyncContext;
    ///
    /// fn print_actor(mut ctx: SyncContext<String>) {
    ///     if let Ok(msg) = ctx.receive_next() {
    ///         println!("Got a message: {msg}");
    ///     } else {
    ///         eprintln!("No message received");
    ///     }
    /// }
    ///
    /// # fn assert_sync_actor<A: heph::actor::SyncActor<RuntimeAccess = ()>>(_: A) { }
    /// # assert_sync_actor(print_actor as fn(_) -> _);
    /// ```
    pub fn receive_next(&mut self) -> Result<M, NoMessages> {
        let waker = self.future_waker();
        waker.block_on(self.inbox.recv()).ok_or(NoMessages)
    }

    /// Block on a [`Future`] waiting for it's completion.
    pub fn block_on<Fut>(&mut self, fut: Fut) -> Fut::Output
    where
        Fut: Future,
    {
        let waker = self.future_waker();
        waker.block_on(fut)
    }

    /// Get mutable access to the runtime this actor is running in.
    pub fn runtime(&mut self) -> &mut RT {
        &mut self.rt
    }

    /// Get access to the runtime this actor is running in.
    pub const fn runtime_ref(&self) -> &RT {
        &self.rt
    }

    /// Returns the [`SyncWaker`] used as [`task::Waker`] in futures.
    fn future_waker(&mut self) -> SyncWaker {
        if let Some(waker) = self.future_waker.as_ref() {
            waker.clone()
        } else {
            let waker = SyncWaker::new();
            self.future_waker = Some(waker.clone());
            waker
        }
    }
}

/// [`task::Waker`] implementation for blocking on [`Future`]s.
// TODO: a `Thread` is already wrapped in an `Arc`, which mean we're double
// `Arc`ing for the `Waker` implementation, try to remove that.
#[derive(Clone, Debug)]
#[doc(hidden)] // Not part of the stable API.
pub struct SyncWaker {
    handle: Thread,
}

impl SyncWaker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        sync_waker_clone,
        sync_waker_wake,
        sync_waker_wake_by_ref,
        sync_waker_drop,
    );

    /// Create a new `SyncWaker`.
    #[doc(hidden)] // Not part of the stable API.
    pub fn new() -> SyncWaker {
        SyncWaker {
            handle: thread::current(),
        }
    }

    /// Poll the `future` until completion, blocking when it can't make
    /// progress.
    #[doc(hidden)] // Not part of the stable API.
    pub fn block_on<Fut>(self: SyncWaker, future: Fut) -> Fut::Output
    where
        Fut: Future,
    {
        let mut future = pin!(future);
        let task_waker = self.into_waker();
        let mut task_ctx = task::Context::from_waker(&task_waker);
        loop {
            match Future::poll(future.as_mut(), &mut task_ctx) {
                Poll::Ready(res) => return res,
                // The waking implementation will `unpark` us.
                Poll::Pending => thread::park(),
            }
        }
    }

    /// Poll the `future` until completion, blocking when it can't make
    /// progress, waiting up to `timeout` time.
    #[doc(hidden)] // Not part of the stable API.
    pub fn block_for<Fut>(self: SyncWaker, future: Fut, timeout: Duration) -> Option<Fut::Output>
    where
        Fut: Future,
    {
        let mut future = pin!(future);
        let task_waker = self.into_waker();
        let mut task_ctx = task::Context::from_waker(&task_waker);

        let start = Instant::now();
        loop {
            match Future::poll(future.as_mut(), &mut task_ctx) {
                Poll::Ready(res) => return Some(res),
                // The waking implementation will `unpark` us.
                Poll::Pending => {
                    let elapsed = start.elapsed();
                    if elapsed > timeout {
                        return None;
                    }

                    thread::park_timeout(timeout - elapsed);
                }
            }
        }
    }

    /// Returns the `SyncWaker` as task `Waker`.
    #[doc(hidden)] // Not part of the stable API.
    pub fn into_waker(self) -> task::Waker {
        let data = self.into_data();
        let raw_waker = RawWaker::new(data, &SyncWaker::VTABLE);
        unsafe { task::Waker::from_raw(raw_waker) }
    }

    /// Returns itself as `task::RawWaker` data.
    fn into_data(self) -> *const () {
        // SAFETY: this is not safe. This only works because `Thread` uses
        // `Pin<Arc<_>>`, which is a pointer underneath.
        unsafe { std::mem::transmute(self) }
    }

    /// Inverse of [`SyncWaker::into_data`].
    ///
    /// # Safety
    ///
    /// `data` MUST be created by [`SyncWaker::into_data`].
    unsafe fn from_data(data: *const ()) -> SyncWaker {
        // SAFETY: inverse of `into_data`, see that for more info.
        unsafe { std::mem::transmute(data) }
    }

    /// Same as [`SyncWaker::from_data`], but returns a reference instead of an
    /// owned `SyncWaker`.
    unsafe fn from_data_ref(data: &*const ()) -> &SyncWaker {
        // SAFETY: inverse of `into_data`, see that for more info, also see
        // `from_data`.
        &*((data as *const *const ()).cast::<SyncWaker>())
    }
}

unsafe fn sync_waker_clone(data: *const ()) -> RawWaker {
    let waker = SyncWaker::from_data_ref(&data);
    let data = waker.clone().into_data();
    RawWaker::new(data, &SyncWaker::VTABLE)
}

unsafe fn sync_waker_wake(data: *const ()) {
    SyncWaker::from_data(data).handle.unpark();
}

unsafe fn sync_waker_wake_by_ref(data: *const ()) {
    SyncWaker::from_data_ref(&data).handle.unpark();
}

unsafe fn sync_waker_drop(data: *const ()) {
    drop(SyncWaker::from_data(data));
}

/// Spawn a synchronous actor.
///
/// This will spawn a new thread to run `actor`, returning the thread's
/// `JoinHandle` and an actor reference.
pub fn spawn_sync_actor<S, A, RT>(
    supervisor: S,
    actor: A,
    arg: A::Argument,
    rt: RT,
) -> io::Result<(thread::JoinHandle<()>, ActorRef<A::Message>)>
where
    S: SyncSupervisor<A> + Send + 'static,
    A: SyncActor<RuntimeAccess = RT> + Send + 'static,
    A::Message: Send + 'static,
    A::Argument: Send + 'static,
    RT: Clone + Send + 'static,
{
    let (inbox, sender, ..) = heph_inbox::Manager::new_small_channel();
    let actor_ref = ActorRef::local(sender);
    let sync_worker = SyncWorker {
        supervisor,
        actor,
        inbox,
    };
    thread::Builder::new()
        .name("Sync actor".to_owned())
        .spawn(move || sync_worker.run(arg, rt))
        .map(|handle| (handle, actor_ref))
}

/// Synchronous worker.
#[derive(Debug)]
struct SyncWorker<S, A: SyncActor> {
    supervisor: S,
    actor: A,
    inbox: inbox::Manager<A::Message>,
}

impl<S, A> SyncWorker<S, A>
where
    S: SyncSupervisor<A>,
    A: SyncActor,
    A::RuntimeAccess: Clone,
{
    /// Run a synchronous actor worker thread.
    fn run(mut self, mut arg: A::Argument, rt: A::RuntimeAccess) {
        let thread = thread::current();
        let name = thread.name().unwrap();
        trace!(name = name; "running synchronous actor");
        loop {
            let receiver = self.inbox.new_receiver().unwrap_or_else(inbox_failure);
            let ctx = SyncContext::new(receiver, rt.clone());
            match self.actor.run(ctx, arg) {
                Ok(()) => break,
                Err(err) => match self.supervisor.decide(err) {
                    SupervisorStrategy::Restart(new_arg) => {
                        trace!(name = name; "restarting synchronous actor");
                        arg = new_arg;
                    }
                    SupervisorStrategy::Stop => break,
                },
            }
        }

        trace!(name = name; "stopping synchronous actor");
    }
}

/// Called when we can't create a new receiver for the sync actor.
#[cold]
fn inbox_failure<T>(_: ReceiverConnected) -> T {
    panic!("failed to create new receiver for synchronous actor's inbox. Was the `SyncContext` leaked?");
}
