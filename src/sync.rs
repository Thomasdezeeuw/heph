//! Synchronous actors.
//!
//! How synchronous actor are run is defined in [`SyncActor`]. An introduction of
//! actors, including the different kinds (synchronous actors included), is given
//! in the [`actor`] module.
//!
//! Synchronous actors are generally run by a `SyncActorRunner` as it handles
//! error and panic handling in combination with the actor's supervisor (see the
//! [`SyncSupervisor`] trait). However as they are often regular function they
//! can also be called directly, though error and panic handling will be up to
//! the caller.
//!
//! [`actor`]: crate::actor
//!
//! # Examples
//!
//! Running a synchronous actor using [`SyncActorRunner`].
//!
//! ```
//! use heph::actor::{actor_fn};
//! use heph::supervisor::NoSupervisor;
//! use heph::sync::{self, SyncActorRunnerBuilder};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a `SyncActorRunner`.
//! let actor = actor_fn(actor);
//! let (sync_actor_runner, actor_ref) = SyncActorRunnerBuilder::new()
//!     .build(NoSupervisor, actor);
//!
//! // The `ActorRef` we can use as expected.
//! actor_ref.try_send("Hello world!")?;
//!
//! // Run the synchronous actor.
//! let arg = ();
//! sync_actor_runner.run(arg);
//! # Ok(())
//! # }
//!
//! fn actor(mut ctx: sync::Context<String>) {
//!     if let Ok(msg) = ctx.receive_next() {
//!         println!("Got a message: {msg}");
//!     } else {
//!         eprintln!("Receive no messages");
//!     }
//! }
//! ```

use std::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::pin::pin;
use std::task::{self, Poll, RawWaker, RawWakerVTable};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};
use std::{io, ptr};

use heph_inbox::Receiver;
use heph_inbox::{self as inbox, ReceiverConnected};
use log::trace;

use crate::actor::private::ActorResult;
use crate::actor::{ActorFn, NoMessages, RecvError};
use crate::actor_ref::ActorRef;
use crate::supervisor::{SupervisorStrategy, SyncSupervisor};

pub use crate::future::InboxSize;

/// Synchronous actor.
///
/// Synchronous actors run on their own thread and therefore can perform
/// synchronous operations such as blocking I/O or time consuming computations.
/// Much like asynchronous [actors] the actor will be supplied with a [context],
/// which can be used for receiving messages. As with asynchronous actors
/// communication is done via message sending, using [actor references].
///
/// The easiest way to implement this trait by using functions, see the [actor
/// module] documentation for an example of this. All functions *pointers* that
/// accept a [`sync::Context`] as argument and return `Result<(), Error>` or
/// `()` implement the `SyncActor` trait. There is also the [`ActorFn`] helper
/// type to implement the trait for any function.
///
/// Synchronous actor can be run using [`SyncActorRunner`].
///
/// [actors]: crate::Actor
/// [context]: Context
/// [actor references]: crate::ActorRef
/// [actor module]: crate::actor
/// [`sync::Context`]: Context
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
    /// The runtime is accessible via the actor's context. See [`sync::Context`]
    /// for more information.
    ///
    /// [`sync::Context`]: Context
    type RuntimeAccess;

    /// Run the synchronous actor.
    fn run(
        &self,
        ctx: Context<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<(), Self::Error>;

    /// Returns the name of the actor.
    ///
    /// The default implementation creates the name based on the name of type of
    /// the actor.
    ///
    /// # Notes
    ///
    /// This uses [`type_name`] under the hood which does not have a stable
    /// output. Like the `type_name` function the default implementation is
    /// provided on a best effort basis.
    ///
    /// When using `my_actor as fn(..) -> _` the will most likely be useless,
    /// consider using [`ActorFn`] which does produce usable names.
    ///
    /// [`type_name`]: std::any::type_name
    fn name() -> &'static str {
        crate::actor::name::<Self>()
    }
}

/// Macro to implement the [`SyncActor`] trait on function pointers.
macro_rules! impl_sync_actor {
    (
        $( ( $( $arg_name: ident : $arg: ident ),* ) ),*
        $(,)?
    ) => {
        $(
            impl<M, RT, $( $arg, )* R> SyncActor for fn(ctx: Context<M, RT>, $( $arg_name: $arg ),*) -> R
            where
                R: ActorResult,
            {
                type Message = M;
                type Argument = ($( $arg ),*);
                type Error = R::Error;
                type RuntimeAccess = RT;

                #[allow(non_snake_case)]
                fn run(&self, ctx: Context<Self::Message, Self::RuntimeAccess>, arg: Self::Argument) -> Result<(), Self::Error> {
                    let ($( $arg ),*) = arg;
                    (self)(ctx, $( $arg ),*).into()
                }
            }

            impl<F, M, RT, $( $arg, )* R> SyncActor for ActorFn<F, M, RT, ($( $arg, )*), R>
            where
                F: Fn(Context<M, RT>, $( $arg ),*) -> R,
                R: ActorResult,
            {
                type Message = M;
                type Argument = ($( $arg ),*);
                type Error = R::Error;
                type RuntimeAccess = RT;

                #[allow(non_snake_case)]
                fn run(&self, ctx: Context<Self::Message, Self::RuntimeAccess>, arg: Self::Argument) -> Result<(), Self::Error> {
                    let ($( $arg ),*) = arg;
                    (self.inner)(ctx, $( $arg ),*).into()
                }

                fn name() -> &'static str {
                    crate::actor::name::<F>()
                }
            }
        )*
    };
}

impl_sync_actor!(());

impl<M, RT, Arg, R> SyncActor for fn(ctx: Context<M, RT>, arg: Arg) -> R
where
    R: ActorResult,
{
    type Message = M;
    type Argument = Arg;
    type Error = R::Error;
    type RuntimeAccess = RT;

    fn run(
        &self,
        ctx: Context<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<(), Self::Error> {
        (self)(ctx, arg).into()
    }
}

impl<F, M, RT, Arg, R> SyncActor for ActorFn<F, M, RT, (Arg,), R>
where
    F: Fn(Context<M, RT>, Arg) -> R,
    R: ActorResult,
{
    type Message = M;
    type Argument = Arg;
    type Error = R::Error;
    type RuntimeAccess = RT;

    #[allow(non_snake_case)]
    fn run(
        &self,
        ctx: Context<Self::Message, Self::RuntimeAccess>,
        arg: Self::Argument,
    ) -> Result<(), Self::Error> {
        ((self.inner)(ctx, arg)).into()
    }

    fn name() -> &'static str {
        crate::actor::name::<F>()
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
pub struct Context<M, RT = ()> {
    inbox: Receiver<M>,
    future_waker: Option<SyncWaker>,
    /// Runtime access.
    rt: RT,
}

impl<M, RT> Context<M, RT> {
    /// Create a new `Context`.
    const fn new(inbox: Receiver<M>, rt: RT) -> Context<M, RT> {
        Context {
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
    /// [`receive_next`]: Context::receive_next
    ///
    /// # Examples
    ///
    /// A synchronous actor that receives a name to greet, or greets the entire
    /// world.
    ///
    /// ```
    /// use heph::sync;
    ///
    /// fn greeter_actor(mut ctx: sync::Context<String>) {
    ///     if let Ok(name) = ctx.try_receive_next() {
    ///         println!("Hello {name}");
    ///     } else {
    ///         println!("Hello world");
    ///     }
    /// }
    ///
    /// # fn assert_sync_actor<A: heph::SyncActor<RuntimeAccess = ()>>(_: A) { }
    /// # assert_sync_actor(heph::actor::actor_fn(greeter_actor));
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
    /// use heph::sync;
    ///
    /// fn print_actor(mut ctx: sync::Context<String>) {
    ///     if let Ok(msg) = ctx.receive_next() {
    ///         println!("Got a message: {msg}");
    ///     } else {
    ///         eprintln!("No message received");
    ///     }
    /// }
    ///
    /// # fn assert_sync_actor<A: heph::SyncActor<RuntimeAccess = ()>>(_: A) { }
    /// # assert_sync_actor(heph::actor::actor_fn(print_actor));
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
                    if elapsed >= timeout {
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
        &*(ptr::from_ref(data).cast())
    }

    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        SyncWaker::clone,
        SyncWaker::wake,
        SyncWaker::wake_by_ref,
        SyncWaker::drop,
    );

    unsafe fn clone(data: *const ()) -> RawWaker {
        let waker = SyncWaker::from_data_ref(&data);
        let data = waker.clone().into_data();
        RawWaker::new(data, &SyncWaker::VTABLE)
    }

    unsafe fn wake(data: *const ()) {
        SyncWaker::from_data(data).handle.unpark();
    }

    unsafe fn wake_by_ref(data: *const ()) {
        SyncWaker::from_data_ref(&data).handle.unpark();
    }

    unsafe fn drop(data: *const ()) {
        drop(SyncWaker::from_data(data));
    }
}

/// Synchronous actor runner.
///
/// See [`SyncActorRunnerBuilder`] for setting various options.
#[derive(Debug)]
pub struct SyncActorRunner<S, A: SyncActor> {
    /// The actor's supervisor used to determine what to do when the actor
    /// returns an error or panics.
    supervisor: S,
    /// The inbox of the actor, used in creating a new [`Context`] if the actor
    /// is restarted.
    inbox: inbox::Manager<A::Message>,
    /// The running actor.
    actor: A,
    /// Runtime access.
    rt: A::RuntimeAccess,
}

impl<S, A> SyncActorRunner<S, A>
where
    S: SyncSupervisor<A>,
    A: SyncActor,
    A::RuntimeAccess: Clone,
{
    /// Create a new `SyncActorRunner`.
    ///
    /// Arguments:
    ///  * `supervisor: S`: is used to handle the actor's errors.
    ///  * `actor: A`: the actor we're running.
    pub fn new(supervisor: S, actor: A) -> (SyncActorRunner<S, A>, ActorRef<A::Message>)
    where
        S: SyncSupervisor<A>,
        A: SyncActor<RuntimeAccess = ()>,
    {
        SyncActorRunnerBuilder::new().build(supervisor, actor)
    }

    /// Run the synchronous actor.
    ///
    /// This handles errors and catches panics, and depending on the supervisor
    /// `S`, restarts the actor if required.
    pub fn run(mut self, mut arg: A::Argument) {
        let name = A::name();
        trace!(name = name; "running synchronous actor");
        loop {
            let receiver = self.inbox.new_receiver().unwrap_or_else(inbox_failure);
            let ctx = Context::new(receiver, self.rt.clone());
            match panic::catch_unwind(AssertUnwindSafe(|| self.actor.run(ctx, arg))) {
                Ok(Ok(())) => break,
                Ok(Err(err)) => match self.supervisor.decide(err) {
                    SupervisorStrategy::Restart(new_arg) => {
                        trace!(name = name; "restarting synchronous actor");
                        arg = new_arg;
                    }
                    SupervisorStrategy::Stop => break,
                },
                Err(panic) => match self.supervisor.decide_on_panic(panic) {
                    SupervisorStrategy::Restart(new_arg) => {
                        trace!(name = name; "restarting synchronous actor after panic");
                        arg = new_arg;
                    }
                    SupervisorStrategy::Stop => break,
                },
            }
        }
        trace!(name = name; "stopping synchronous actor");
    }

    /// Returns the name of the actor.
    ///
    /// Based on the [`SyncActor::name`] implementation.
    pub fn name() -> &'static str {
        A::name()
    }
}

/// Called when we can't create a new receiver for the sync actor.
#[cold]
fn inbox_failure<T>(_: ReceiverConnected) -> T {
    panic!("failed to create new receiver for synchronous actor's inbox. Was the `sync::Context` leaked?");
}

/// Builder for [`SyncActorRunner`].
///
/// This allows setting various options.
#[derive(Debug)]
#[must_use = "call `build` to finish building the `SyncActorRunner`"]
pub struct SyncActorRunnerBuilder<RT = ()> {
    rt: RT,
    inbox_size: InboxSize,
}

impl SyncActorRunnerBuilder {
    /// Create a new `SyncActorRunnerBuilder`, which allows for the creation of
    /// `SyncActorRunner` with more options.
    pub const fn new() -> SyncActorRunnerBuilder {
        SyncActorRunnerBuilder {
            rt: (),
            inbox_size: InboxSize::DEFAULT,
        }
    }
}

impl<RT> SyncActorRunnerBuilder<RT> {
    /// Returns the runtime access used by the actor.
    pub fn rt(&self) -> &RT {
        &self.rt
    }

    /// Set the runtime access used by the actor.
    ///
    /// `rt: RT`: is used to get access to the runtime, it defaults to the unit
    /// type (`()`) in case it's not needed. It needs to be `Clone` as it's
    /// passed to the actor and after is needed for possible restarts.
    pub fn with_rt<RT2>(self, rt: RT2) -> SyncActorRunnerBuilder<RT2>
    where
        RT: Clone,
    {
        SyncActorRunnerBuilder {
            rt,
            inbox_size: self.inbox_size,
        }
    }

    /// Returns the size of the actor's inbox.
    pub fn inbox_size(&self) -> InboxSize {
        self.inbox_size
    }

    /// Set the size of the actor's inbox.
    pub fn with_inbox_size(mut self, size: InboxSize) -> Self {
        self.inbox_size = size;
        self
    }

    /// Create a new `SyncActorRunner`.
    ///
    /// Arguments:
    ///  * `supervisor: S`: is used to handle the actor's errors.
    ///  * `actor: A`: the actor we're running.
    pub fn build<S, A>(
        self,
        supervisor: S,
        actor: A,
    ) -> (SyncActorRunner<S, A>, ActorRef<A::Message>)
    where
        S: SyncSupervisor<A>,
        A: SyncActor<RuntimeAccess = RT>,
        RT: Clone,
    {
        let (inbox, sender, ..) = inbox::Manager::new_channel(self.inbox_size.get());
        let actor_ref = ActorRef::local(sender);
        let sync_worker = SyncActorRunner {
            supervisor,
            inbox,
            actor,
            rt: self.rt,
        };
        (sync_worker, actor_ref)
    }

    /// Spawn a synchronous actor.
    ///
    /// This will spawn a new thread to run `actor`, returning the thread's
    /// `JoinHandle` and an actor reference.
    pub fn spawn<S, A>(
        self,
        supervisor: S,
        actor: A,
        argument: A::Argument,
    ) -> io::Result<(thread::JoinHandle<()>, ActorRef<A::Message>)>
    where
        S: SyncSupervisor<A> + Send + 'static,
        A: SyncActor<RuntimeAccess = RT> + Send + 'static,
        A::Message: Send + 'static,
        A::Argument: Send + 'static,
        RT: Clone + Send + 'static,
    {
        let (sync_worker, actor_ref) = self.build(supervisor, actor);
        thread::Builder::new()
            .name(A::name().to_owned())
            .spawn(move || sync_worker.run(argument))
            .map(|handle| (handle, actor_ref))
    }
}
