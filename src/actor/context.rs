//! Module containing the `Context` and related types.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};
use std::{fmt, io};

use mio::{event, Interest, Token};

use crate::actor::message_select::First;
use crate::actor_ref::ActorRef;
use crate::inbox::{Inbox, InboxRef};
use crate::rt::{ActorOptions, ProcessId, RuntimeRef, SharedRuntimeInternal};
use crate::{NewActor, Supervisor};

/// The context in which an actor is executed.
///
/// This context can be used for a number of things including receiving
/// messages and getting access to the runtime.
///
/// The `actor::Context` comes in two flavours:
/// * [`ThreadLocal`] (default) is the optimised version, but doesn't allow the
///   actor to move between threads. Actor started with
///   [`RuntimeRef::try_spawn_local`] will get this flavour of context.
/// * [`ThreadSafe`] is the flavour that allows the actor to be moved between
///   threads. Actor started with [`RuntimeRef::try_spawn`] will get this
///   flavour of context.
#[derive(Debug)]
pub struct Context<M, K = ThreadLocal> {
    /// Process id of the actor, used as `Token` in registering things, e.g.
    /// a `TcpStream`, with `mio::Poll`.
    pid: ProcessId,
    /// Inbox of the actor, shared between this and zero or more actor
    /// references. It's owned by the context, the actor references only have a
    /// weak reference.
    ///
    /// This field is public because it is used by `tcp::Server`, as we don't
    /// need entire context there.
    pub(crate) inbox: Inbox<M>,
    /// Reference to `inbox` above, used to create `ActorRef`s to this actor.
    inbox_ref: InboxRef<M>,
    /// Kind of the context.
    kind: K,
}

/// Provides a thread-local actor context.
///
/// This is optimised version of [`ThreadSafe`], but doesn't allow the actor to
/// move between threads.
///
/// See [`actor::Context`] for more information.
///
/// [`actor::Context`]: crate::actor::Context
pub struct ThreadLocal {
    /// A reference to the runtime, used to get access to `mio::Poll`.
    runtime_ref: RuntimeRef,
}

/// Provides a thread-safe actor context.
///
/// See [`actor::Context`] for more information.
///
/// [`actor::Context`]: crate::actor::Context
pub struct ThreadSafe {
    runtime_ref: Arc<SharedRuntimeInternal>,
}

impl<M> Context<M, ThreadLocal> {
    /// Create a new local `actor::Context`.
    pub(crate) fn new_local(
        pid: ProcessId,
        inbox: Inbox<M>,
        inbox_ref: InboxRef<M>,
        runtime_ref: RuntimeRef,
    ) -> Context<M, ThreadLocal> {
        Context {
            pid,
            inbox,
            inbox_ref,
            kind: ThreadLocal { runtime_ref },
        }
    }
}

impl<M> Context<M, ThreadSafe> {
    /// Create a new local `actor::Context`.
    pub(crate) fn new_shared(
        pid: ProcessId,
        inbox: Inbox<M>,
        inbox_ref: InboxRef<M>,
        runtime_ref: Arc<SharedRuntimeInternal>,
    ) -> Context<M, ThreadSafe> {
        Context {
            pid,
            inbox,
            inbox_ref,
            kind: ThreadSafe { runtime_ref },
        }
    }
}

impl<M, C> Context<M, C> {
    /// Attempt to receive the next message.
    ///
    /// This will attempt to receive next message if one is available. If the
    /// actor wants to wait until a message is received
    /// [`actor::Context::receive_next`] can be used, which returns a
    /// `Future<Output = M>`.
    ///
    /// [`actor::Context::receive_next`]: crate::actor::Context::receive_next
    ///
    /// # Examples
    ///
    /// An actor that receives a name to greet, or greets the entire world.
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use heph::actor;
    ///
    /// async fn greeter_actor(mut ctx: actor::Context<String>) -> Result<(), !> {
    ///     if let Some(name) = ctx.try_receive_next() {
    ///         println!("Hello: {}", name);
    ///     } else {
    ///         println!("Hello world");
    ///     }
    ///     Ok(())
    /// }
    ///
    /// # // Use the `greeter_actor` function to silence dead code warning.
    /// # drop(greeter_actor);
    /// ```
    pub fn try_receive_next(&mut self) -> Option<M> {
        self.inbox.receive_next()
    }

    /*
    /// Attempt to receive a specific message.
    ///
    /// This will attempt to receive a message using message selection, if one
    /// is available. If the actor wants to wait until a message is received
    /// [`actor::Context::receive`] can be used, which returns a
    /// `Future<Output = M>`.
    ///
    /// [`actor::Context::receive`]: crate::actor::Context::receive
    ///
    /// # Examples
    ///
    /// In this example the actor first handles priority messages and only after
    /// all of those are handled it will handle normal messages.
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use heph::actor;
    ///
    /// #[derive(Debug)]
    /// enum Message {
    ///     Priority(String),
    ///     Normal(String),
    /// }
    ///
    /// impl Message {
    ///     /// Whether or not the message is a priority message.
    ///     fn is_priority(&self) -> bool {
    ///         match self {
    ///             Message::Priority(_) => true,
    ///             _ => false,
    ///         }
    ///     }
    /// }
    ///
    /// async fn actor(mut ctx: actor::Context<Message>) -> Result<(), !> {
    ///     // First we handle priority messages.
    ///     while let Some(priority_msg) = ctx.try_receive(Message::is_priority) {
    ///         println!("Priority message: {:?}", priority_msg);
    ///     }
    ///
    ///     // After that we handle normal messages.
    ///     while let Some(msg) = ctx.try_receive_next() {
    ///         println!("Normal message: {:?}", msg);
    ///     }
    ///     Ok(())
    /// }
    ///
    /// # // Use the actor and all message variants to silence dead code
    /// # // warnings.
    /// # drop(actor);
    /// # drop(Message::Priority("".to_owned()));
    /// # drop(Message::Normal("".to_owned()));
    /// ```
    pub fn try_receive<S>(&mut self, mut selector: S) -> Option<M>
    where
        S: MessageSelector<M>,
    {
        self.inbox.receive(&mut selector)
    }
    */

    /// Receive the next message.
    ///
    /// This returns a [`Future`] that will complete once a message is ready.
    ///
    /// # Examples
    ///
    /// An actor that await a message and prints it.
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use heph::actor;
    ///
    /// async fn print_actor(mut ctx: actor::Context<String>) -> Result<(), !> {
    ///     let msg = ctx.receive_next().await;
    ///     println!("Got a message: {}", msg);
    ///     Ok(())
    /// }
    ///
    /// # // Use the `print_actor` function to silence dead code warning.
    /// # drop(print_actor);
    /// ```
    ///
    /// Same as the example above, but this actor will only wait for a limited
    /// amount of time.
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use std::time::Duration;
    ///
    /// use futures_util::future::FutureExt;
    /// use futures_util::select;
    /// use heph::actor;
    /// use heph::timer::Timer;
    ///
    /// async fn print_actor(mut ctx: actor::Context<String>) -> Result<(), !> {
    ///     // Create a timer, this will be ready once the timeout has
    ///     // passed.
    ///     let mut timeout = Timer::timeout(&mut ctx, Duration::from_millis(100)).fuse();
    ///     // Create a future to receive a message.
    ///     let mut msg_future = ctx.receive_next().fuse();
    ///
    ///     // Now let them race!
    ///     // This is basically a match statement for futures, whichever
    ///     // future is ready first will be the winner and we'll take that
    ///     // branch.
    ///     select! {
    ///         msg = msg_future => println!("Got a message: {}", msg),
    ///         _ = timeout => println!("No message"),
    ///     };
    ///
    ///     Ok(())
    /// }
    ///
    /// # // Use the `print_actor` function to silence dead code warning.
    /// # drop(print_actor);
    /// ```
    pub fn receive_next<'ctx>(&'ctx mut self) -> ReceiveMessage<'ctx, M> {
        ReceiveMessage {
            inbox: &mut self.inbox,
            selector: First,
        }
    }

    /*
    /// Receive a message.
    ///
    /// This returns a [`Future`] that will complete once a message is ready.
    ///
    /// See [`actor::Context::try_receive`] and [`MessageSelector`] for examples
    /// on how to use the message selector and see
    /// [`actor::Context::receive_next`] for an example that uses the same
    /// `Future` this method returns.
    ///
    /// [`actor::Context::try_receive`]: crate::actor::Context::try_receive
    /// [`actor::Context::receive_next`]: crate::actor::Context::receive_next
    pub fn receive<'ctx, S>(&'ctx mut self, selector: S) -> ReceiveMessage<'ctx, M, S>
    where
        S: MessageSelector<M>,
    {
        ReceiveMessage {
            inbox: &mut self.inbox,
            selector,
        }
    }
    */

    /*
    /// Attempt to peek the next message.
    pub fn try_peek_next(&mut self) -> Option<M>
    where
        M: Clone,
    {
        self.inbox.peek_next()
    }

    /// Attempt to peek a specific message.
    pub fn try_peek<S>(&mut self, mut selector: S) -> Option<M>
    where
        S: MessageSelector<M>,
        M: Clone,
    {
        self.inbox.peek(&mut selector)
    }

    /// Peek at the next message.
    pub fn peek_next<'ctx>(&'ctx mut self) -> PeekMessage<'ctx, M>
    where
        M: Clone,
    {
        PeekMessage {
            inbox: &mut self.inbox,
            selector: First,
        }
    }

    /// Peek a message.
    ///
    /// This returns a future that will complete once a message is ready. The
    /// message will be cloned, which means that the next call to [`receive`] or
    /// [`peek`] will return the same message.
    ///
    /// [`receive`]: Context::receive
    /// [`peek`]: Context::peek
    pub fn peek<'ctx, S>(&'ctx mut self, selector: S) -> PeekMessage<'ctx, M, S>
    where
        S: MessageSelector<M>,
        M: Clone,
    {
        PeekMessage {
            inbox: &mut self.inbox,
            selector,
        }
    }
    */

    /// Returns a reference to this actor.
    pub fn actor_ref(&mut self) -> ActorRef<M> {
        ActorRef::from_inbox(self.inbox_ref.clone())
    }

    /// Get the pid of this actor.
    pub(crate) fn pid(&self) -> ProcessId {
        self.pid
    }

    pub(crate) fn kind(&mut self) -> &mut C {
        &mut self.kind
    }
}

impl<M> Context<M, ThreadLocal> {
    /// Get a reference to the runtime this actor is running in.
    pub fn runtime(&mut self) -> &mut RuntimeRef {
        &mut self.kind.runtime_ref
    }
}

impl<M> Context<M, ThreadSafe> {
    /// Attempts to spawn a thread-safe actor.
    ///
    /// See [`RuntimeRef::try_spawn`].
    pub fn try_spawn<Sv, NA>(
        &mut self,
        supervisor: Sv,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, NA::Error>
    where
        Sv: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Context = ThreadSafe> + Sync + Send + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        self.kind
            .runtime_ref
            .spawn_setup(supervisor, new_actor, move |_, _| Ok(arg), options)
            .map_err(|err| match err {
                AddActorError::NewActor(err) => err,
                AddActorError::<_, !>::ArgFn(_) => unreachable!(),
            })
    }

    /// Spawn an thread-safe actor.
    ///
    /// See [`RuntimeRef::spawn`].
    pub fn spawn<Sv, NA>(
        &mut self,
        supervisor: Sv,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<NA::Message>
    where
        Sv: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Error = !, Context = ThreadSafe> + Sync + Send + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        self.kind
            .runtime_ref
            .spawn_setup(supervisor, new_actor, move |_, _| Ok(arg), options)
            .unwrap_or_else(|_: AddActorError<!, !>| unreachable!())
    }
}

/// Implementation detail to support [`ThreadSafe`] and [`ThreadLocal`] contexts
/// within the same implementation.
// public because it used in trait bound for methods like `UdpSocket::bind`.
pub trait ContextKind {
    /// Create a new [`task::Waker`].
    fn new_task_waker(runtime_ref: &mut RuntimeRef, pid: ProcessId) -> task::Waker;

    /// Creates a new context.
    fn new_context<M>(
        pid: ProcessId,
        inbox: Inbox<M>,
        inbox_ref: InboxRef<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> Context<M, Self>
    where
        Self: Sized;

    /// Registers the `source` at the correct `Poll` instance using `token` and
    /// `interest`.
    fn register<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized;

    /// Reregisters the `source` at the correct `Poll` instance using `token` and
    /// `interest`.
    fn reregister<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized;
}

impl ContextKind for ThreadLocal {
    fn new_task_waker(runtime_ref: &mut RuntimeRef, pid: ProcessId) -> task::Waker {
        runtime_ref.new_local_task_waker(pid)
    }

    fn new_context<M>(
        pid: ProcessId,
        inbox: Inbox<M>,
        inbox_ref: InboxRef<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> Context<M, ThreadLocal> {
        Context::new_local(pid, inbox, inbox_ref, runtime_ref.clone())
    }

    fn register<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.runtime_ref.register(source, token, interest)
    }

    fn reregister<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.runtime_ref.reregister(source, token, interest)
    }
}

impl ContextKind for ThreadSafe {
    fn new_task_waker(runtime_ref: &mut RuntimeRef, pid: ProcessId) -> task::Waker {
        runtime_ref.new_shared_task_waker(pid)
    }

    fn new_context<M>(
        pid: ProcessId,
        inbox: Inbox<M>,
        inbox_ref: InboxRef<M>,
        runtime_ref: &mut RuntimeRef,
    ) -> Context<M, ThreadSafe> {
        Context::new_shared(pid, inbox, inbox_ref, runtime_ref.clone_shared())
    }

    fn register<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.runtime_ref.register(source, token, interest)
    }

    fn reregister<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.runtime_ref.reregister(source, token, interest)
    }
}

/// Implementation detail to support [`ThreadSafe`] and [`ThreadLocal`] contexts
/// within the same implementation.
// public because it used in trait bound for methods like `UdpSocket::bind`.
pub trait Spawnable<S, NA>
where
    Self: Sized,
    S: Supervisor<NA>,
    NA: NewActor<Context = Self>,
{
    /// Spawn a new actor.
    fn spawn<ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        ArgFn: FnOnce(ProcessId, &mut Self) -> Result<NA::Argument, ArgFnE>;
}

/// Internal error returned by spawning a actor.
#[derive(Debug)]
pub enum AddActorError<NewActorE, ArgFnE> {
    /// Calling `NewActor::new` actor resulted in an error.
    NewActor(NewActorE),
    /// Calling the argument function resulted in an error.
    ArgFn(ArgFnE),
}

impl<S, NA> Spawnable<S, NA> for ThreadLocal
where
    Self: Sized,
    S: Supervisor<NA> + 'static,
    NA: NewActor<Context = Self> + 'static,
    NA::Actor: 'static,
{
    fn spawn<ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        ArgFn: FnOnce(ProcessId, &mut Self) -> Result<NA::Argument, ArgFnE>,
    {
        self.runtime_ref
            .try_spawn_local_setup(supervisor, new_actor, arg_fn, options)
    }
}

impl<S, NA> Spawnable<S, NA> for ThreadSafe
where
    Self: Sized,
    S: Supervisor<NA> + Send + Sync + 'static,
    NA: NewActor<Context = Self> + Send + Sync + 'static,
    NA::Actor: Send + Sync + 'static,
    NA::Message: Send,
{
    fn spawn<ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        ArgFn: FnOnce(ProcessId, &mut Self) -> Result<NA::Argument, ArgFnE>,
    {
        self.runtime_ref
            .spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}

impl fmt::Debug for ThreadLocal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThreadLocal")
    }
}

impl fmt::Debug for ThreadSafe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ThreadSafe")
    }
}

/// Future to receive a single message.
///
/// The implementation behind and [`actor::Context::receive_next`].
///
/// [`actor::Context::receive_next`]: crate::actor::Context::receive_next
#[derive(Debug)]
pub struct ReceiveMessage<'ctx, M, S = First> {
    inbox: &'ctx mut Inbox<M>,
    selector: S,
}

/*
impl<'ctx, M, S> Future for ReceiveMessage<'ctx, M, S>
where
    S: MessageSelector<M> + Unpin,
{
    type Output = M;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let ReceiveMessage {
            ref mut inbox,
            ref mut selector,
        } = self.deref_mut();
        match inbox.receive(selector) {
            Some(msg) => Poll::Ready(msg),
            // Wakeup notifications are done when adding to the mailbox.
            None => Poll::Pending,
        }
    }
}
*/

impl<'ctx, M> Future for ReceiveMessage<'ctx, M, First> {
    type Output = M;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        match self.inbox.receive_next() {
            Some(msg) => Poll::Ready(msg),
            // Wakeup notifications are done when adding to the mailbox.
            None => Poll::Pending,
        }
    }
}

/*
/// Future to peek a single message.
///
/// The implementation behind [`actor::Context::peek`] and
/// [`actor::Context::peek_next`].
///
/// [`actor::Context::peek`]: crate::actor::Context::peek
/// [`actor::Context::peek_next`]: crate::actor::Context::peek_next
#[derive(Debug)]
pub struct PeekMessage<'ctx, M, S = First> {
    inbox: &'ctx mut Inbox<M>,
    selector: S,
}

impl<'ctx, M, S> Future for PeekMessage<'ctx, M, S>
where
    S: MessageSelector<M> + Unpin,
    M: Clone,
{
    type Output = M;

    fn poll(mut self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let PeekMessage {
            ref mut inbox,
            ref mut selector,
        } = self.deref_mut();
        match inbox.peek(selector) {
            Some(msg) => Poll::Ready(msg),
            // Wakeup notifications are done when adding to the mailbox.
            None => Poll::Pending,
        }
    }
}
*/
