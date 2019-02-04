//! Module containing the `ActorContext` and related types.

use std::future::Future;
use std::pin::Pin;
use std::task::{LocalWaker, Poll};

use crate::actor_ref::{ActorRef, Local};
use crate::mailbox::MailBox;
use crate::scheduler::ProcessId;
use crate::system::ActorSystemRef;
use crate::util::Shared;

/// The context in which an actor is executed.
///
/// This context can be used for a number of things including receiving
/// messages and getting access to the running actor system.
#[derive(Debug)]
pub struct ActorContext<M> {
    /// Process id of the actor, used as `EventedId` in registering things, e.g.
    /// a `TcpStream`, with the system poller.
    pid: ProcessId,
    /// A reference to the actor system, used to get access to the system
    /// poller.
    system_ref: ActorSystemRef,
    /// Inbox of the actor, shared between this and zero or more actor
    /// references. It's owned by the context, the actor references only have a
    /// weak reference.
    ///
    /// This field is public because it is used by `TcpListener`, as we don't
    /// need entire context there.
    pub(crate) inbox: Shared<MailBox<M>>,
}

impl<M> ActorContext<M> {
    /// Create a new `ActorContext`.
    pub(crate) const fn new(pid: ProcessId, system_ref: ActorSystemRef, inbox: Shared<MailBox<M>>) -> ActorContext<M> {
        ActorContext {
            pid,
            system_ref,
            inbox,
        }
    }

    /// Attempt to receive a message.
    ///
    /// This will attempt to receive a message if one is available. If the actor
    /// wants to wait until a message is received [`ActorContext::receive`] can
    /// be used, which returns a `Future<Output = M>`.
    ///
    /// # Examples
    ///
    /// An actor that receives a name to greet or greets the entire world.
    ///
    /// ```
    /// #![feature(async_await, futures_api, never_type)]
    ///
    /// use heph::actor::ActorContext;
    ///
    /// async fn greeter_actor(mut ctx: ActorContext<String>) -> Result<(), !> {
    ///     if let Some(name) = ctx.try_receive_next() {
    ///         println!("Hello: {}", name);
    ///     } else {
    ///         println!("Hello world");
    ///     }
    ///     Ok(())
    /// }
    /// ```
    pub fn try_receive_next(&mut self) -> Option<M> {
        self.inbox.borrow_mut().receive_next()
    }

    /// Receive a message.
    ///
    /// This returns a future that will complete once a message is ready.
    ///
    /// # Examples
    ///
    /// An actor that receives messages and prints them in a loop.
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api, never_type)]
    ///
    /// use heph::actor::ActorContext;
    ///
    /// async fn print_actor(mut ctx: ActorContext<String>) -> Result<(), !> {
    ///     loop {
    ///         let msg = await!(ctx.receive_next());
    ///         println!("Got a message: {}", msg);
    ///     }
    /// }
    /// ```
    ///
    /// Same as the example above, but this actor will only wait for a limited
    /// amount of time.
    ///
    /// ```ignore
    /// #![feature(async_await, await_macro, futures_api, never_type)]
    ///
    /// use std::time::Duration;
    ///
    /// use futures_util::future::FutureExt;
    /// use futures_util::select;
    /// use heph::actor::ActorContext;
    /// use heph::timer::Timer;
    ///
    /// async fn print_actor(mut ctx: ActorContext<String>) -> Result<(), !> {
    ///     loop {
    ///         // Create a timer, this will be ready once the timeout has
    ///         // passed.
    ///         let mut timeout = Timer::timeout(&mut ctx, Duration::from_millis(100)).fuse();
    ///         // Create a future to receive a message.
    ///         let mut msg_future = ctx.receive_next().fuse();
    ///
    ///         // Now let them race!
    ///         // This is basically a match statement for futures, whichever
    ///         // future is ready first will be the winner and we'll take that
    ///         // branch.
    ///         select! {
    ///             msg = msg_future => println!("Got a message: {}", msg),
    ///             _ = timeout => {
    ///                 println!("Getting impatient!");
    ///                 continue;
    ///             },
    ///         };
    ///     }
    /// }
    /// ```
    pub fn receive_next<'ctx>(&'ctx mut self) -> ReceiveMessage<'ctx, M> {
        ReceiveMessage {
            inbox: &mut self.inbox,
        }
    }

    /// Returns an actor reference to this actor.
    pub fn actor_ref(&mut self) -> ActorRef<M> {
        ActorRef::<M, Local>::new(self.inbox.downgrade())
    }

    /// Get a reference to the actor system this actor is running in.
    pub fn system_ref(&mut self) -> &mut ActorSystemRef {
        &mut self.system_ref
    }

    /// Get the pid of this actor.
    pub(crate) fn pid(&self) -> ProcessId {
        self.pid
    }
}

/// The implementation behind [`ActorContext.receive`].
///
/// [`ActorContext.receive`]: struct.ActorContext.html#method.receive
#[derive(Debug)]
pub struct ReceiveMessage<'ctx, M> {
    inbox: &'ctx mut Shared<MailBox<M>>,
}

impl<'ctx, M> Future for ReceiveMessage<'ctx, M> {
    type Output = M;

    fn poll(mut self: Pin<&mut Self>, _waker: &LocalWaker) -> Poll<Self::Output> {
        match self.inbox.borrow_mut().receive_next() {
            Some(msg) => Poll::Ready(msg),
            // Wakeup notifications are done when adding to the mailbox.
            None => Poll::Pending,
        }
    }
}
