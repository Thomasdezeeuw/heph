//! Module containing actor references.
//!
//! An actor reference is a generic reference to an actor that can run on the
//! same thread, another thread on the same machine or even running remotely.
//!
//! ## Sending messages
//!
//! All types of actor references have a [`send`] method. These methods don't
//! block, even the remote actor reference, but the method doesn't provided a
//! lot of guarantees. What [`send`] does is asynchronously add the message to
//! the queue of messages for the actor.
//!
//! In case of the local actor reference this can be done directly. But for
//! machine local actor references the message must first be send across thread
//! bounds before being added to the actor's message queue. Remote actor
//! references even need to send this message across a network, a lot can go
//! wrong here.
//!
//! If guarantees are needed that a message is received or processed the
//! receiving actor should send back an acknowledgment that the message is
//! received and/or processed correctly.
//!
//! Other then the `send` method the `<<=` operator can be used to send
//! messages, which does the same thing as `send` but with nicer syntax. The
//! following example shows how messages can be send using this operator. It
//! uses a local actor reference but it's the same for all flavours.
//!
//! [`send`]: crate::actor_ref::ActorRef::send
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::supervisor::NoSupervisor;
//! use heph::{actor, RuntimeError, ActorOptions, Runtime};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     Runtime::new()
//!         .with_setup(|mut runtime_ref| {
//!             // Spawn the actor.
//!             let new_actor = actor as fn(_) -> _;
//!             let mut actor_ref = runtime_ref.spawn(NoSupervisor, new_actor, (),
//!                 ActorOptions::default());
//!
//!             // Now we can use the reference to send the actor a message.
//!             actor_ref <<= "Hello world".to_owned();
//!             // Above is the same as:
//!             // let _ = actor_ref.send("Hello world".to_owned());
//!
//!             Ok(())
//!         })
//!         .start()
//! }
//!
//! /// Our actor.
//! async fn actor(mut ctx: actor::Context<String>) -> Result<(), !> {
//!     let msg = ctx.receive_next().await;
//!     println!("got message: {}", msg);
//!     Ok(())
//! }
//! ```
//!
//! ## Sharing actor references
//!
//! All actor references can be cloned, which is the easiest way to share them.
//!
//! The example below shows how an local actor reference is cloned to send a
//! message to the same actor, but it is the same for all types of references.
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::supervisor::NoSupervisor;
//! use heph::{actor, RuntimeError, ActorOptions, Runtime};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     Runtime::new()
//!         .with_setup(|mut runtime_ref| {
//!             let new_actor = actor as fn(_) -> _;
//!             let mut actor_ref = runtime_ref.spawn(NoSupervisor, new_actor, (),
//!                 ActorOptions::default());
//!
//!             // To create another actor reference we can simply clone the
//!             // first one.
//!             let mut second_actor_ref = actor_ref.clone();
//!
//!             // Now we can use both references to send a message.
//!             actor_ref <<= "Hello world".to_owned();
//!             second_actor_ref <<= "Bye world".to_owned();
//!
//!             Ok(())
//!         })
//!         .start()
//! }
//!
//! /// Our actor.
//! async fn actor(mut ctx: actor::Context<String>) -> Result<(), !> {
//!     let msg = ctx.receive_next().await;
//!     println!("First message: {}", msg);
//!
//!     let msg = ctx.receive_next().await;
//!     println!("Second message: {}", msg);
//!     Ok(())
//! }
//! ```

use std::convert::TryFrom;
use std::error::Error;
use std::fmt;
use std::ops::ShlAssign;
use std::sync::Arc;

use crossbeam_channel::Sender;

use crate::actor;
use crate::inbox::InboxRef;

mod rpc;
pub use rpc::{NoResponse, Rpc, RpcMessage, RpcResponse};

#[cfg(test)]
mod tests;

/// Trait to erase the original message type of the actor reference.
trait MappedActorRef<M> {
    fn mapped_send(&self, msg: M) -> Result<(), SendError>;
}

/// Trait to erase the original message type of the actor reference.
trait TryMappedActorRef<M> {
    fn try_mapped_send(&self, msg: M) -> Result<(), SendError>;
}

/// Non-local actor reference.
///
/// An actor reference reference can be used to send messages to an actor, for
/// more details see the [module] documentation.
///
/// [module]: crate::actor_ref
pub struct ActorRef<M> {
    kind: ActorRefKind<M>,
}

enum ActorRefKind<M> {
    /// Reference to an actor that might be on another thread, but on the same
    /// node.
    Node(InboxRef<M>),
    /// Reference to a synchronous actor.
    Sync(Sender<M>),
    /// Reference that maps the message to a different type first.
    Mapped(Arc<dyn MappedActorRef<M>>),
    /// Reference that attempts to map the message to a different type first.
    TryMapped(Arc<dyn TryMappedActorRef<M>>),
}

// We know that `Node` and `Sync` variants are `Send` and `Sync` and since the
// `Mapped` and `TryMapped` variants are only one of those two so are those
// variants, which makes the entire `ActorRefKind` `Send` and `Sync`, as long as
// `M` is `Send` (as we would be sending the message across thread bounds).
unsafe impl<M: Send> Send for ActorRefKind<M> {}
unsafe impl<M: Send> Sync for ActorRefKind<M> {}

impl<M> ActorRef<M> {
    /// Create a new `ActorRef` for an actor using `inbox_ref`.
    pub(crate) const fn from_inbox(inbox_ref: InboxRef<M>) -> ActorRef<M> {
        ActorRef {
            kind: ActorRefKind::Node(inbox_ref),
        }
    }

    /// Create a new `ActorRef` for a synchronous actor.
    pub(crate) const fn for_sync_actor(sender: Sender<M>) -> ActorRef<M> {
        ActorRef {
            kind: ActorRefKind::Sync(sender),
        }
    }

    /// Asynchronously send a message to the actor.
    ///
    /// Some types of actor references can detect errors in sending a message,
    /// however not all actor references can. This means that even if this
    /// methods returns `Ok` it does **not** mean that the message is guaranteed
    /// to be delivered to or handled by the actor.
    ///
    /// See [Sending messages] for more details.
    ///
    /// [Sending messages]: index.html#sending-messages
    pub fn send<Msg>(&self, msg: Msg) -> Result<(), SendError>
    where
        Msg: Into<M>,
    {
        #[cfg(any(test, feature = "test"))]
        {
            if crate::test::should_lose_msg() {
                log::debug!("dropping message on purpose");
                return Ok(());
            }
        }

        let msg = msg.into();
        use ActorRefKind::*;
        match &self.kind {
            Node(inbox_ref) => inbox_ref.try_send(msg).map_err(|_| SendError),
            Sync(sender) => sender.try_send(msg).map_err(|_err| SendError),
            Mapped(actor_ref) => actor_ref.mapped_send(msg),
            TryMapped(actor_ref) => actor_ref.try_mapped_send(msg),
        }
    }

    /// Make a Remote Procedure Call (RPC).
    ///
    /// This will send the `request` to the actor and returns a [`Rpc`]
    /// [`Future`] that will return a response (of type `Res`), or an error if
    /// the receiving actor didn't respond.
    pub fn rpc<CM, Req, Res>(
        &mut self,
        ctx: &mut actor::Context<CM>,
        request: Req,
    ) -> Result<Rpc<Res>, SendError>
    where
        M: From<RpcMessage<Req, Res>>,
    {
        let pid = ctx.pid();
        let waker = ctx.runtime().new_waker(pid);
        let (msg, rpc) = Rpc::new(waker, request);
        self.send(msg).map(|()| rpc)
    }

    /// Changes the message type of the actor reference.
    ///
    /// Before sending the message this will first change the message into a
    /// different type. This is useful when you need to send to different types
    /// of actors (using different message types) from a central location.
    ///
    /// # Notes
    ///
    /// This conversion is **not** cheap, it requires an allocation so use with
    /// caution when it comes to performance sensitive code.
    ///
    /// Prefer to clone an existing mapped `ActorRef` over creating a new one as
    /// that can reuse the allocation mentioned above.
    pub fn map<Msg>(self) -> ActorRef<Msg>
    where
        M: From<Msg>,
        Self: 'static,
    {
        ActorRef {
            kind: ActorRefKind::Mapped(Arc::new(self)),
        }
    }

    /// Much like [`map`], but uses the [`TryFrom`] trait.
    ///
    /// This creates a new local actor reference that attempts to map from one
    /// message type to another before sending. This is useful when you need to
    /// send to different types of actors from a central location.
    ///
    /// [`map`]: ActorRef::map
    ///
    /// # Notes
    ///
    /// Errors converting from one message type to another are turned into
    /// [`SendError`]s.
    ///
    /// This conversion is **not** cheap, it requires an allocation so use with
    /// caution when it comes to performance sensitive code.
    ///
    /// Prefer to clone an existing mapped `ActorRef` over creating a new one as
    /// that can reuse the allocation mentioned above.
    pub fn try_map<Msg>(self) -> ActorRef<Msg>
    where
        M: TryFrom<Msg>,
        Self: 'static,
    {
        ActorRef {
            kind: ActorRefKind::TryMapped(Arc::new(self)),
        }
    }
}

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> ActorRef<M> {
        use ActorRefKind::*;
        ActorRef {
            kind: match &self.kind {
                Node(inbox_ref) => Node(inbox_ref.clone()),
                Sync(sender) => Sync(sender.clone()),
                Mapped(actor_ref) => Mapped(actor_ref.clone()),
                TryMapped(actor_ref) => TryMapped(actor_ref.clone()),
            },
        }
    }
}

impl<M, Msg> ShlAssign<Msg> for ActorRef<M>
where
    Msg: Into<M>,
{
    fn shl_assign(&mut self, msg: Msg) {
        let _ = self.send(msg);
    }
}

impl<M, Msg> ShlAssign<Msg> for &ActorRef<M>
where
    Msg: Into<M>,
{
    fn shl_assign(&mut self, msg: Msg) {
        let _ = self.send(msg);
    }
}

impl<M> fmt::Debug for ActorRef<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ActorRef")
    }
}

impl<M, Msg> MappedActorRef<Msg> for ActorRef<M>
where
    M: From<Msg>,
{
    fn mapped_send(&self, msg: Msg) -> Result<(), SendError> {
        self.send(msg)
    }
}

impl<M, Msg> TryMappedActorRef<Msg> for ActorRef<M>
where
    M: TryFrom<Msg>,
{
    fn try_mapped_send(&self, msg: Msg) -> Result<(), SendError> {
        M::try_from(msg)
            .map_err(|_msg| SendError)
            .and_then(|msg| self.send(msg))
    }
}

/// Error returned when sending a message fails.
///
/// The reason why the sending of the message failed is unspecified.
#[derive(Copy, Clone, Debug)]
pub struct SendError;

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unable to send message")
    }
}

impl Error for SendError {}
