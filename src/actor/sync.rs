//! Module containing the types for synchronous actors.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{self, Poll};
use std::thread::{self, Thread};

use inbox::Receiver;

use crate::actor::{NoMessages, RecvError};
use crate::trace::{self, Trace};

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
/// [module level]: crate::actor
///
/// Synchronous actor can only be spawned before starting the runtime, see
/// [`Runtime::spawn_sync_actor`].
///
/// # Signal handling
///
/// [Process signals] are automatically send to synchronous actors if the
/// [`Message`] type implements the required trait bound to convert the actor
/// reference into mapped actor reference, see [`ActorRef::try_map`].
///
/// ```
/// use heph::actor::SyncContext;
/// # use heph::actor_ref::ActorRef;
/// use heph::from_message;
/// use heph::rt::{self, Runtime, Signal};
/// use heph::spawn::SyncActorOptions;
/// use heph::supervisor::NoSupervisor;
///
/// fn main() -> Result<(), rt::Error> {
///     // Spawning synchronous actor works slightly differently than spawning
///     // regular (asynchronous) actors. Mainly, synchronous actors need to be
///     // spawned before the runtime is started.
///     let mut runtime = Runtime::new()?;
///
///     // Spawn a new synchronous actor, returning an actor reference to it.
///     let actor = actor as fn(_);
///     let options = SyncActorOptions::default().with_name("My actor".to_string());
///     let actor_ref = runtime.spawn_sync_actor(NoSupervisor, actor, (), options)?;
///
///     // Just like with any actor reference we can send the actor a message.
///     actor_ref.try_send("Hello world".to_string()).unwrap();
///     # let actor_ref: ActorRef<Signal> = actor_ref.try_map();
///     # actor_ref.try_send(Signal::Interrupt).unwrap();
///
///     // And now we start the runtime.
///     runtime.start()
/// }
///
/// enum Message {
///     Print(String),
///     Signal(Signal),
/// }
///
/// from_message!(Message::Print(String));
/// // This implementation allows the sync actor to receive process signals.
/// from_message!(Message::Signal(Signal));
///
/// fn actor(mut ctx: SyncContext<Message>) {
///     while let Ok(msg) = ctx.receive_next() {
///         match msg {
///             Message::Print(msg) => println!("Got a message: {}", msg),
///             Message::Signal(_signal) => break,
///         }
///     }
/// }
/// ```
///
/// [Process signals]: crate::rt::Signal
/// [`Message`]: SyncActor::Message
/// [`ActorRef::try_map`]: crate::actor_ref::ActorRef::try_map
///
/// # Panics
///
/// Panics are not caught and will **not** be returned to the actor's
/// supervisor. If a synchronous actor panics it will bring down the entire
/// runtime.
///
/// [actors]: crate::Actor
/// [context]: SyncContext
/// [actor references]: crate::ActorRef
/// [`Runtime::spawn_sync_actor`]: crate::Runtime::spawn_sync_actor
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

/// Macro to implement the [`SyncActor`] trait on function pointers.
macro_rules! impl_sync_actor {
    (
        $( ( $( $arg: ident ),* ) ),*
        $(,)*
    ) => {
        $(
            impl<M, E, $( $arg ),*> SyncActor for fn(SyncContext<M>, $( $arg ),*) -> Result<(), E> {
                type Message = M;
                type Argument = ($( $arg ),*);
                type Error = E;

                #[allow(non_snake_case)]
                fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
                    let ($( $arg ),*) = arg;
                    (self)(ctx, $( $arg ),*)
                }
            }

            impl<M, $( $arg ),*> SyncActor for fn(SyncContext<M>, $( $arg ),*) {
                type Message = M;
                type Argument = ($( $arg ),*);
                type Error = !;

                #[allow(non_snake_case)]
                fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
                    let ($( $arg ),*) = arg;
                    Ok((self)(ctx, $( $arg ),*))
                }
            }
        )*
    };
}

impl<M, E, Arg> SyncActor for fn(SyncContext<M>, Arg) -> Result<(), E> {
    type Message = M;
    type Argument = Arg;
    type Error = E;

    fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
        (self)(ctx, arg)
    }
}

impl<M, Arg> SyncActor for fn(SyncContext<M>, Arg) {
    type Message = M;
    type Argument = Arg;
    type Error = !;

    fn run(&self, ctx: SyncContext<Self::Message>, arg: Self::Argument) -> Result<(), Self::Error> {
        #[allow(clippy::unit_arg)]
        Ok((self)(ctx, arg))
    }
}

impl_sync_actor!(
    (),
    // NOTE: we don't want a single argument into tuple form so we implement
    // that manually above.
    (Arg1, Arg2),
    (Arg1, Arg2, Arg3),
    (Arg1, Arg2, Arg3, Arg4),
    (Arg1, Arg2, Arg3, Arg4, Arg5),
);

/// The context in which a synchronous actor is executed.
///
/// This context can be used for a number of things including receiving
/// messages.
#[derive(Debug)]
pub struct SyncContext<M> {
    inbox: Receiver<M>,
    future_waker: Option<Arc<SyncWaker>>,
    trace_log: Option<trace::Log>,
}

impl<M> SyncContext<M> {
    /// Create a new `SyncContext`.
    pub(crate) fn new(inbox: Receiver<M>, trace_log: Option<trace::Log>) -> SyncContext<M> {
        SyncContext {
            inbox,
            future_waker: None,
            trace_log,
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
    ///         println!("Hello {}", name);
    ///     } else {
    ///         println!("Hello world");
    ///     }
    /// }
    ///
    /// # fn assert_sync_actor<A: heph::actor::SyncActor>(_: A) { }
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
    ///         println!("Got a message: {}", msg);
    ///     } else {
    ///         eprintln!("No message received");
    ///     }
    /// }
    ///
    /// # fn assert_sync_actor<A: heph::actor::SyncActor>(_: A) { }
    /// # assert_sync_actor(print_actor as fn(_) -> _);
    /// ```
    pub fn receive_next(&mut self) -> Result<M, NoMessages> {
        let waker = self.future_waker();
        waker.block_on(self.inbox.recv()).ok_or(NoMessages)
    }

    /// Block on a [`Future`] waiting for it's completion.
    ///
    /// # Limitations
    ///
    /// Any [`Future`] returned by a type that is [bound] to an actor **cannot**
    /// be used by this function. Those types use specialised wake-up mechanisms
    /// bypassing the `Future`'s [`task`] system. This currently includes all
    /// types in the [`net`] and [`timer`] modules.
    ///
    /// [bound]: crate::actor::Bound
    /// [`net`]: crate::net
    /// [`timer`]: crate::timer
    pub fn block_on<Fut>(&mut self, fut: Fut) -> Fut::Output
    where
        Fut: Future,
    {
        let waker = self.future_waker();
        waker.block_on(fut)
    }

    /// Returns the [`SyncWaker`] used as [`task::Waker`] in futures.
    fn future_waker(&mut self) -> Arc<SyncWaker> {
        if let Some(waker) = self.future_waker.as_ref() {
            waker.clone()
        } else {
            let waker = Arc::new(SyncWaker {
                handle: thread::current(),
            });
            self.future_waker = Some(waker.clone());
            waker
        }
    }
}

impl<M> Trace for SyncContext<M> {
    fn start_trace(&self) -> Option<trace::EventTiming> {
        trace::start(&self.trace_log)
    }

    fn finish_trace(
        &mut self,
        timing: Option<trace::EventTiming>,
        description: &str,
        attributes: &[(&str, &dyn trace::AttributeValue)],
    ) {
        trace::finish(&mut self.trace_log, timing, 1, description, attributes);
    }
}

/// [`task::Waker`] implementation for blocking on [`Future`]s.
// TODO: a `Thread` is already wrapped in an `Arc`, which mean we're double
// `Arc`ing for the `Waker` implementation, try to remove that.
#[derive(Debug)]
struct SyncWaker {
    handle: Thread,
}

impl task::Wake for SyncWaker {
    fn wake(self: Arc<Self>) {
        self.handle.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.handle.unpark();
    }
}

impl SyncWaker {
    /// Poll the `fut`ure until completion, blocking when it can't make
    /// progress.
    fn block_on<Fut>(self: Arc<SyncWaker>, fut: Fut) -> Fut::Output
    where
        Fut: Future,
    {
        // Pin the `Future` to stack.
        let mut future = fut;
        let mut future = unsafe { Pin::new_unchecked(&mut future) };

        let task_waker = task::Waker::from(self);
        let mut task_ctx = task::Context::from_waker(&task_waker);
        loop {
            match Future::poll(future.as_mut(), &mut task_ctx) {
                Poll::Ready(res) => return res,
                // The waking implementation will `unpark` us.
                Poll::Pending => thread::park(),
            }
        }
    }
}
