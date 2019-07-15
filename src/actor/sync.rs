//! Module containing the types for synchronous actors.
//!
//! # Examples
//!
//! Spawn and run a synchronous actor.
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::actor::sync::SyncContext;
//! use heph::supervisor::NoSupervisor;
//! use heph::system::{ActorSystem, RuntimeError};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     // Spawning synchronous actor works slightly different from spawning
//!     // regular (asynchronous) actors. Mainly, synchronous actors need to be
//!     // spawned before the system is run.
//!     let mut system = ActorSystem::new();
//!
//!     // Spawn a new synchronous actor, returning an actor reference to it.
//!     let actor = actor as fn(_, _) -> _;
//!     let mut actor_ref = system.spawn_sync_actor(NoSupervisor, actor, "Bye")?;
//!
//!     // Just like with any actor reference we can send the actor a message.
//!     actor_ref <<= "Hello world".to_string();
//!
//!     // And now we run the system.
//!     system.run()
//! }
//!
//! fn actor(mut ctx: SyncContext<String>, exit_msg: &'static str) -> Result<(), !> {
//!     if let Ok(msg) = ctx.receive_next() {
//!         println!("Got a message: {}", msg);
//!     } else {
//!         eprintln!("Receive no messages");
//!     }
//!     println!("{}", exit_msg);
//!     Ok(())
//! }
//! ```

use std::collections::VecDeque;
use std::fmt;
use std::ptr::NonNull;

use crossbeam_channel::Receiver;

use crate::actor::message_select::{MessageSelector, Messages};

/// Synchronous actor.
///
/// Synchronous actor run on its own thread and therefore can perform
/// synchronous operations such as blocking I/O. Much like regular [actors] the
/// actor will be supplied with a [context], which can be used for receiving
/// messages. As with regular actors communication is done via message sending,
/// using [actor references].
///
/// The easiest way to implement this trait by using regular functions, see the
/// [module level] documentation for an example of this.
///
/// [module level]: crate::actor::sync
///
/// Synchronous actor can only be spawned before running the actor system, see
/// [`ActorSystem::spawn_sync_actor`].
///
/// # Panics
///
/// Panics are not caught, thus bringing down the actor's thread, panics will
/// **not** be returned to the actor's supervisor.
///
/// [actors]: crate::Actor
/// [context]: SyncContext
/// [actor references]: crate::ActorRef
/// [`ActorSystem::spawn_sync_actor`]:
/// crate::system::ActorSystem::spawn_sync_actor
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
    /// [supervisor]: crate::supervisor
    type Error;

    /// Run the synchronous actor.
    fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error>;
}

impl<M, E> SyncActor for fn(ctx: SyncContext<M>) -> Result<(), E> {
    type Message = M;
    type Argument = ();
    type Error = E;
    fn run(
        &self,
        ctx: SyncContext<Self::Message>,
        _arg: Self::Argument,
    ) -> Result<(), Self::Error> {
        (self)(ctx)
    }
}

impl<M, E, Arg> SyncActor for fn(ctx: SyncContext<M>, arg: Arg) -> Result<(), E> {
    type Message = M;
    type Argument = Arg;
    type Error = E;
    fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
        (self)(ctx, arg)
    }
}

impl<M, E, Arg1, Arg2> SyncActor
    for fn(ctx: SyncContext<M>, arg1: Arg1, arg2: Arg2) -> Result<(), E>
{
    type Message = M;
    type Argument = (Arg1, Arg2);
    type Error = E;
    fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
        (self)(ctx, arg.0, arg.1)
    }
}

impl<M, E, Arg1, Arg2, Arg3> SyncActor
    for fn(ctx: SyncContext<M>, arg1: Arg1, arg2: Arg2, arg3: Arg3) -> Result<(), E>
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3);
    type Error = E;
    fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
        (self)(ctx, arg.0, arg.1, arg.2)
    }
}

impl<M, E, Arg1, Arg2, Arg3, Arg4> SyncActor
    for fn(ctx: SyncContext<M>, arg1: Arg1, arg2: Arg2, arg3: Arg3, arg4: Arg4) -> Result<(), E>
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3, Arg4);
    type Error = E;
    fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
        (self)(ctx, arg.0, arg.1, arg.2, arg.3)
    }
}

impl<M, E, Arg1, Arg2, Arg3, Arg4, Arg5> SyncActor
    for fn(
        ctx: SyncContext<M>,
        arg1: Arg1,
        arg2: Arg2,
        arg3: Arg3,
        arg4: Arg4,
        arg5: Arg5,
    ) -> Result<(), E>
{
    type Message = M;
    type Argument = (Arg1, Arg2, Arg3, Arg4, Arg5);
    type Error = E;
    fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
        (self)(ctx, arg.0, arg.1, arg.2, arg.3, arg.4)
    }
}

/// The context in which an synchronous actor is executed.
///
/// This context can be used for a number of things including receiving
/// messages.
#[derive(Debug)]
#[repr(transparent)]
pub struct SyncContext<M> {
    ptr: NonNull<SyncContextData<M>>,
}

/// Data in `SyncContext`.
pub(crate) struct SyncContextData<M> {
    /// The messages in the inbox.
    messages: VecDeque<M>,
    inbox: Receiver<M>,
}

impl<M> SyncContextData<M> {
    /// Create a new `SyncContextData`.
    pub(crate) fn new(inbox: Receiver<M>) -> SyncContextData<M> {
        SyncContextData {
            messages: VecDeque::new(),
            inbox,
        }
    }
}

impl<M> SyncContext<M> {
    /// Create a new `SyncContext` based on a pointer to `SyncContextData`.
    ///
    /// # Unsafety
    ///
    /// The caller must ensure the context data stays alive as long as
    /// `SyncContext` is alive.
    pub(crate) const unsafe fn new(ptr: NonNull<SyncContextData<M>>) -> SyncContext<M> {
        SyncContext { ptr }
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
    /// #![feature(never_type)]
    ///
    /// use heph::actor::sync::SyncContext;
    ///
    /// fn greeter_actor(mut ctx: SyncContext<String>) -> Result<(), !> {
    ///     if let Some(name) = ctx.try_receive_next() {
    ///         println!("Hello: {}", name);
    ///     } else {
    ///         println!("Hello world");
    ///     }
    ///     Ok(())
    /// }
    ///
    /// # fn assert_sync_actor<A: heph::actor::sync::SyncActor>(_a: A) { }
    /// # assert_sync_actor(greeter_actor as fn(_) -> _);
    /// ```
    pub fn try_receive_next(&mut self) -> Option<M> {
        self.receive_messages();
        self.data().messages.pop_front()
    }

    /// Attempt to receive a specific message.
    ///
    /// This will attempt to receive a message using message selection, if one
    /// is available. If the actor wants to wait until a message is available
    /// [`receive`] can be used.
    ///
    /// [`receive`]: SyncContext::receive
    ///
    /// # Examples
    ///
    /// In this example the actor first handles priority messages and only after
    /// all of those are handled it will handle normal messages.
    ///
    /// ```
    /// #![feature(never_type)]
    ///
    /// use heph::actor::sync::SyncContext;
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
    /// fn actor(mut ctx: SyncContext<Message>) -> Result<(), !> {
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
    /// # fn assert_sync_actor<A: heph::actor::sync::SyncActor>(_a: A) { }
    /// # assert_sync_actor(actor as fn(_) -> _);
    /// # // Use all message variants to silence dead code warnings.
    /// # drop(Message::Priority("".to_owned()));
    /// # drop(Message::Normal("".to_owned()));
    /// ```
    pub fn try_receive<S>(&mut self, mut selector: S) -> Option<M>
    where
        S: MessageSelector<M>,
    {
        self.receive_messages();
        selector
            .select(Messages::new(&self.data().messages))
            .and_then(|selection| self.data().messages.remove(selection.0))
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
    /// #![feature(async_await, await_macro, never_type)]
    ///
    /// use heph::actor::sync::SyncContext;
    ///
    /// fn print_actor(mut ctx: SyncContext<String>) -> Result<(), !> {
    ///     if let Ok(msg) = ctx.receive_next() {
    ///         println!("Got a message: {}", msg);
    ///     } else {
    ///         eprintln!("No message received");
    ///     }
    ///     Ok(())
    /// }
    ///
    /// # fn assert_sync_actor<A: heph::actor::sync::SyncActor>(_a: A) { }
    /// # assert_sync_actor(print_actor as fn(_) -> _);
    /// ```
    pub fn receive_next(&mut self) -> Result<M, NoMessages> {
        self.receive_messages_blocking()?;
        self.data().messages.pop_front().ok_or(NoMessages)
    }

    /// Receive a message.
    ///
    /// Receives a message using messages selection. If no messages are
    /// currently available it will block until a message becomes available or
    /// until all actor references (that reference this actor) are dropped.
    pub fn receive<S>(&mut self, mut selector: S) -> Result<M, NoMessages>
    where
        S: MessageSelector<M>,
    {
        self.receive_messages_blocking()?;
        let data = self.data();
        selector
            .select(Messages::new(&data.messages))
            .and_then(|selection| data.messages.remove(selection.0))
            .ok_or(NoMessages)
    }

    /// Peek at the next message.
    pub fn peek_next(&mut self) -> Result<M, NoMessages>
    where
        M: Clone,
    {
        self.receive_messages_blocking()?;
        self.data().messages.front().cloned().ok_or(NoMessages)
    }

    /// Peek a message.
    ///
    /// Peek at a message using messages selection. The message will be cloned,
    /// which means that the next call to [`receive`] or [`peek`] will return
    /// the same message. If no messages are currently available it will block
    /// until a message becomes available.
    ///
    /// [`receive`]: SyncContext::receive
    /// [`peek`]: SyncContext::peek
    pub fn peek<S>(&mut self, mut selector: S) -> Result<M, NoMessages>
    where
        S: MessageSelector<M>,
        M: Clone,
    {
        self.receive_messages_blocking()?;
        let data = self.data();
        selector
            .select(Messages::new(&data.messages))
            .and_then(|selection| data.messages.get(selection.0).cloned())
            .ok_or(NoMessages)
    }

    /// Receive all messages, if any, and add them to the message queue.
    fn receive_messages(&mut self) {
        let data = self.data();
        while let Ok(msg) = data.inbox.try_recv() {
            data.messages.push_back(msg);
        }
    }

    /// Receive all messages, if any, and add them to the message queue.
    ///
    /// If no messages are available this will block until one becomes
    /// available.
    fn receive_messages_blocking(&mut self) -> Result<(), NoMessages> {
        self.receive_messages();

        let data = self.data();
        if data.messages.is_empty() {
            // If no messages are available we'll block until we receive one.
            let msg = data.inbox.recv().map_err(|_| NoMessages)?;
            data.messages.push_back(msg);
        }
        Ok(())
    }

    /// Get access to the context data.
    fn data(&mut self) -> &mut SyncContextData<M> {
        unsafe {
            // This is safe because the creator of `SyncContext` must ensure the
            // context doesn't outlive the context data.
            self.ptr.as_mut()
        }
    }
}

/// Returned when a synchronous actor has no messages in its inbox and no
/// references to the actor exists.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct NoMessages;

impl fmt::Display for NoMessages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("no messages in inbox")
    }
}
