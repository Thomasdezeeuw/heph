//! Module containing the `Context` and related types.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};
use std::time::Instant;
use std::{fmt, io};

use inbox::{Receiver, RecvValue};
use mio::{event, Interest, Token};

use crate::actor::{AddActorError, PrivateSpawn, Spawn};
use crate::actor_ref::ActorRef;
use crate::rt::{self, shared, ActorOptions, ProcessId, RuntimeRef};
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
pub struct Context<M, RT = ThreadLocal> {
    /// Process id of the actor, used as `Token` in registering things, e.g.
    /// a `TcpStream`, with `mio::Poll`.
    pid: ProcessId,
    /// Inbox of the actor, shared between this and zero or more actor
    /// references.
    ///
    /// This field is public because it is used by `TcpServer`, as we don't need
    /// entire context there.
    pub(crate) inbox: Receiver<M>,
    /// Runtime access.
    rt: RT,
}

/// Provides a thread-local actor context.
///
/// This is an optimised version of [`ThreadSafe`], but doesn't allow the actor
/// to move between threads.
///
/// See [`actor::Context`] for more information.
///
/// [`actor::Context`]: crate::actor::Context
pub struct ThreadLocal {
    runtime_ref: RuntimeRef,
}

/// Provides a thread-safe actor context.
///
/// See [`actor::Context`] for more information.
///
/// [`actor::Context`]: crate::actor::Context
pub struct ThreadSafe {
    runtime_ref: Arc<shared::RuntimeInternals>,
}

impl<M, RT> Context<M, RT> {
    /// Attempt to receive the next message.
    ///
    /// This will attempt to receive next message if one is available. If the
    /// actor wants to wait until a message is received [`receive_next`] can be
    /// used, which returns a `Future<Output = M>`.
    ///
    /// [`receive_next`]: Context::receive_next
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
    /// async fn greeter_actor(mut ctx: actor::Context<String>) {
    ///     if let Ok(name) = ctx.try_receive_next() {
    ///         println!("Hello: {}", name);
    ///     } else {
    ///         println!("Hello world");
    ///     }
    /// }
    ///
    /// # // Use the `greeter_actor` function to silence dead code warning.
    /// # drop(greeter_actor);
    /// ```
    pub fn try_receive_next(&mut self) -> Result<M, RecvError> {
        self.inbox.try_recv().map_err(RecvError::from)
    }

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
    /// async fn print_actor(mut ctx: actor::Context<String>) {
    ///     if let Ok(msg) = ctx.receive_next().await {
    ///         println!("Got a message: {}", msg);
    ///     }
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
    /// use heph::actor;
    /// use heph::timer::Timer;
    /// use heph::util::either;
    ///
    /// async fn print_actor(mut ctx: actor::Context<String>) {
    ///     // Create a timer, this will be ready once the timeout has
    ///     // passed.
    ///     let timeout = Timer::after(&mut ctx, Duration::from_millis(100));
    ///     // Create a future to receive a message.
    ///     let msg_future = ctx.receive_next();
    ///
    ///     // Now let them race!
    ///     match either(msg_future, timeout).await {
    ///         Ok(Ok(msg)) => println!("Got a message: {}", msg),
    ///         Ok(Err(_)) => println!("No message"),
    ///         Err(_) => println!("Timed out receiving message"),
    ///     }
    /// }
    ///
    /// # // Use the `print_actor` function to silence dead code warning.
    /// # drop(print_actor);
    /// ```
    #[allow(clippy::needless_lifetimes)]
    pub fn receive_next<'ctx>(&'ctx mut self) -> ReceiveMessage<'ctx, M> {
        ReceiveMessage {
            recv: self.inbox.recv(),
        }
    }

    /// Returns a reference to this actor.
    pub fn actor_ref(&self) -> ActorRef<M> {
        ActorRef::local(self.inbox.new_sender())
    }

    /// Get the pid of this actor.
    pub(crate) const fn pid(&self) -> ProcessId {
        self.pid
    }

    /// Sets the waker of the inbox to `waker`.
    pub(crate) fn register_inbox_waker(&mut self, waker: &task::Waker) {
        let _ = self.inbox.register_waker(waker);
    }
}

impl<M> Context<M, ThreadLocal> {
    /// Create a new local `actor::Context`.
    pub(crate) const fn new_local(
        pid: ProcessId,
        inbox: Receiver<M>,
        runtime_ref: RuntimeRef,
    ) -> Context<M, ThreadLocal> {
        Context {
            pid,
            inbox,
            rt: ThreadLocal { runtime_ref },
        }
    }

    /// Get a reference to the runtime this actor is running in.
    pub fn runtime(&mut self) -> &mut RuntimeRef {
        &mut self.rt.runtime_ref
    }
}

impl<M> Context<M, ThreadSafe> {
    /// Create a new local `actor::Context`.
    pub(crate) const fn new_shared(
        pid: ProcessId,
        inbox: Receiver<M>,
        runtime_ref: Arc<shared::RuntimeInternals>,
    ) -> Context<M, ThreadSafe> {
        Context {
            pid,
            inbox,
            rt: ThreadSafe { runtime_ref },
        }
    }

    /// Attempt to spawn a new thead-safe actor.
    ///
    /// See the [`Spawn`] trait for more information.
    pub fn try_spawn<S, NA>(
        &mut self,
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
    {
        Spawn::try_spawn(self, supervisor, new_actor, arg, options)
    }

    /// Spawn a new thead-safe actor.
    ///
    /// See the [`Spawn`] trait for more information.
    pub fn spawn<S, NA>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg: NA::Argument,
        options: ActorOptions,
    ) -> ActorRef<NA::Message>
    where
        S: Supervisor<NA> + Send + Sync + 'static,
        NA: NewActor<Error = !, RuntimeAccess = ThreadSafe> + Sync + Send + 'static,
        NA::Actor: Send + Sync + 'static,
        NA::Message: Send,
    {
        Spawn::spawn(self, supervisor, new_actor, arg, options)
    }
}

impl<M, S, NA> Spawn<S, NA, ThreadLocal> for Context<M, ThreadLocal> {}

impl<M, S, NA> PrivateSpawn<S, NA, ThreadLocal> for Context<M, ThreadLocal> {
    fn try_spawn_setup<ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<RuntimeAccess = ThreadLocal> + 'static,
        NA::Actor: 'static,
        ArgFn: FnOnce(&mut Context<NA::Message, ThreadLocal>) -> Result<NA::Argument, ArgFnE>,
    {
        self.rt
            .runtime_ref
            .try_spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}

impl<M, S, NA> Spawn<S, NA, ThreadSafe> for Context<M, ThreadSafe>
where
    S: Send + Sync,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + Sync,
    NA::Actor: Send + Sync,
    NA::Message: Send,
{
}

impl<M, S, NA> PrivateSpawn<S, NA, ThreadSafe> for Context<M, ThreadSafe>
where
    S: Send + Sync,
    NA: NewActor<RuntimeAccess = ThreadSafe> + Send + Sync,
    NA::Actor: Send + Sync,
    NA::Message: Send,
{
    fn try_spawn_setup<ArgFn, ArgFnE>(
        &mut self,
        supervisor: S,
        new_actor: NA,
        arg_fn: ArgFn,
        options: ActorOptions,
    ) -> Result<ActorRef<NA::Message>, AddActorError<NA::Error, ArgFnE>>
    where
        S: Supervisor<NA> + 'static,
        NA: NewActor<RuntimeAccess = ThreadSafe> + 'static,
        NA::Actor: 'static,
        ArgFn: FnOnce(&mut Context<NA::Message, ThreadSafe>) -> Result<NA::Argument, ArgFnE>,
    {
        self.rt
            .runtime_ref
            .spawn_setup(supervisor, new_actor, arg_fn, options)
    }
}

impl rt::PrivateAccess for ThreadLocal {
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

    fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.runtime_ref.add_deadline(pid, deadline)
    }

    fn cpu(&self) -> Option<usize> {
        self.runtime_ref.cpu()
    }
}

impl rt::PrivateAccess for ThreadSafe {
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

    fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.runtime_ref.add_deadline(pid, deadline)
    }

    fn cpu(&self) -> Option<usize> {
        None
    }
}

impl<M, RT> rt::PrivateAccess for Context<M, RT>
where
    RT: rt::PrivateAccess,
{
    fn register<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.rt.register(source, token, interest)
    }

    fn reregister<S>(&mut self, source: &mut S, token: Token, interest: Interest) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.rt.reregister(source, token, interest)
    }

    fn add_deadline(&mut self, pid: ProcessId, deadline: Instant) {
        self.rt.add_deadline(pid, deadline)
    }

    fn cpu(&self) -> Option<usize> {
        self.rt.cpu()
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

/// Error returned in case receiving a value from an actor's inbox fails.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RecvError {
    /// Inbox is empty.
    Empty,
    /// All [`ActorRef`]s  are disconnected and the inbox is empty.
    Disconnected,
}

impl RecvError {
    pub(crate) const fn from(err: inbox::RecvError) -> RecvError {
        match err {
            inbox::RecvError::Empty => RecvError::Empty,
            inbox::RecvError::Disconnected => RecvError::Disconnected,
        }
    }
}

/// Future to receive a single message.
///
/// The implementation behind and [`actor::Context::receive_next`].
///
/// [`actor::Context::receive_next`]: crate::actor::Context::receive_next
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ReceiveMessage<'ctx, M> {
    recv: RecvValue<'ctx, M>,
}

impl<'ctx, M> Future for ReceiveMessage<'ctx, M> {
    type Output = Result<M, NoMessages>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.recv)
            .poll(ctx)
            .map(|r| r.ok_or(NoMessages))
    }
}

/// Returned when an actor's inbox has no messages and no references to the
/// actor exists.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct NoMessages;

impl fmt::Display for NoMessages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no messages in inbox")
    }
}
