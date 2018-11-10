//! Module containing the `ActorContext` and related types.

use std::future::Future;
use std::pin::Pin;
use std::task::{LocalWaker, Poll};

use crate::actor_ref::LocalActorRef;
use crate::mailbox::MailBox;
use crate::process::ProcessId;
use crate::system::ActorSystemRef;
use crate::util::Shared;

/// The context in which an actor is executed.
///
/// This context can be used for a number of things including receiving
/// messages and spawning new actors.
#[derive(Debug)]
pub struct ActorContext<M> {
    /// Process id of the actor, used as `EventedId` in registering things, e.g.
    /// a `TcpStream`, with the system poller.
    pid: ProcessId,
    /// A reference to the actor system, used to get access to the system
    /// poller.
    system_ref: ActorSystemRef,
    /// Inbox of the actor, shared between this and zero or more
    /// `LocalActorRef`s. It's owned by the context, the actor references only
    /// have a weak reference.
    inbox: Shared<MailBox<M>>,
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

    /// Receive a message.
    ///
    /// This returns a future that will complete once a message is ready.
    ///
    /// # Example
    ///
    /// An actor that receives messages and print them in a loop.
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api, never_type)]
    ///
    /// use heph::actor::{actor_factory, ActorContext};
    ///
    /// async fn print_actor(mut ctx: ActorContext<String>, item: ()) -> Result<(), !> {
    ///     loop {
    ///         let msg = await!(ctx.receive());
    ///         println!("Got a message: {}", msg);
    ///     }
    /// }
    /// ```
    ///
    /// Same as the example above, but this actor will only wait for a limited
    /// amount of time.
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api, pin, never_type)]
    ///
    /// use std::time::Duration;
    ///
    /// use futures_util::select;
    /// use heph::actor::{actor_factory, ActorContext};
    /// use heph::timer::Timer;
    ///
    /// async fn print_actor(mut ctx: ActorContext<String>, item: ()) -> Result<(), !> {
    ///     loop {
    ///         // Create a timer, this will be ready once the timeout has
    ///         // passed.
    ///         let mut timeout = Timer::timeout(&mut ctx, Duration::from_millis(100));
    ///         // Create a future to receive a message.
    ///         let mut msg = ctx.receive();
    ///
    ///         // Now let them race!
    ///         // This is basically a match statement for futures, whichever
    ///         // future is ready first will be the winner and we'll take that
    ///         // branch.
    ///         select! {
    ///             msg => println!("Got a message: {}", msg),
    ///             timeout => {
    ///                 println!("Getting impatient!");
    ///                 continue;
    ///             },
    ///         };
    ///     }
    /// }
    /// ```
    pub fn receive<'ctx>(&'ctx mut self) -> ReceiveFuture<'ctx, M> {
        ReceiveFuture {
            inbox: &mut self.inbox,
        }
    }

    /// Returns an actor reference that references itself.
    pub fn myself(&mut self) -> LocalActorRef<M> {
        LocalActorRef::new(self.inbox.downgrade())
    }

    /// Get a reference to the actor system this actor is running in.
    pub(crate) fn system_ref(&mut self) -> &mut ActorSystemRef {
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
pub struct ReceiveFuture<'ctx, M: 'ctx> {
    inbox: &'ctx mut Shared<MailBox<M>>,
}

impl<'ctx, M> Future for ReceiveFuture<'ctx, M> {
    type Output = M;

    fn poll(mut self: Pin<&mut Self>, _waker: &LocalWaker) -> Poll<Self::Output> {
        match self.inbox.borrow_mut().receive() {
            Some(msg) => Poll::Ready(msg),
            // Wakeup Notifications are done when adding to the mailbox.
            None => Poll::Pending,
        }
    }
}
