//! Module containing the `ActorContext` and related types.

use std::future::Future;
use std::mem::PinMut;
use std::task::{Context, Poll};

use crate::actor_ref::LocalActorRef;
use crate::mailbox::MailBox;
use crate::process::ProcessId;
use crate::system::ActorSystemRef;
use crate::util::Shared;

/// The context in which an actor is executed.
///
/// This context can be used for a number of things including receiving
/// messages.
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
    /// This future only returns once a message is ready. See the examples below
    /// for a way to deal with this.
    ///
    /// # Example
    ///
    /// An actor that receives messages and print them in a loop.
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api, never_type)]
    ///
    /// use heph::actor::{ActorContext, actor_factory};
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
    /// use heph::actor::{ActorContext, actor_factory};
    /// use heph::timer::Timer;
    /// use futures_util::select;
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
    ///         let msg = select! {
    ///             msg => msg,
    ///             timeout => {
    ///                 println!("Getting impatient!");
    ///                 continue;
    ///             },
    ///         };
    ///
    ///         println!("Got a message: {}", msg);
    ///     }
    /// }
    /// ```
    pub fn receive<'ctx>(&'ctx mut self) -> impl Future<Output = M> + 'ctx {
        ReceiveFuture {
            inbox: &mut self.inbox,
        }
    }

    /// Returns an actor reference of itself.
    pub fn myself(&mut self) -> LocalActorRef<M> {
        LocalActorRef::new(self.inbox.downgrade())
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

#[derive(Debug)]
struct ReceiveFuture<'ctx, M: 'ctx> {
    inbox: &'ctx mut Shared<MailBox<M>>,
}

impl<'ctx, M> Future for ReceiveFuture<'ctx, M> {
    type Output = M;

    fn poll(mut self: PinMut<Self>, _ctx: &mut Context) -> Poll<Self::Output> {
        match self.inbox.borrow_mut().receive() {
            Some(msg) => Poll::Ready(msg),
            None => Poll::Pending,
        }
    }
}
