//! Module containing actor references.
//!
//! An actor reference is a generic reference to an actor that can run on the
//! same thread, another thread on the same node or even running remotely.
//!
//! # Sending messages
//!
//! The primary function of actor references is sending messages. This can be
//! done by using the [`send`] or [`try_send`] methods. These methods don't
//! block, even the remote actor reference, but the methods don't provided a lot
//! of guarantees. It doesn't even guarantee the order in which the messages
//! arive. What [`send`] does is asynchronously add the message to the queue of
//! messages for the actor.
//!
//! In case of thread-local actor reference this can be done directly. But for
//! thread-safe actor references the message must first be send across thread
//! bounds before being added to the actor's message queue. Remote actor
//! references even need to send this message across a network, a lot can go
//! wrong here.
//!
//! If guarantees are needed that a message is received or processed the
//! receiving actor should send back an acknowledgment that the message is
//! received and/or processed correctly.
//!
//! [`send`]: ActorRef::send
//! [`try_send`]: ActorRef::try_send
//!
//! This example shows a simple actor that prints all the messages it receives.
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::supervisor::NoSupervisor;
//! use heph::{actor, rt, ActorOptions, Runtime};
//!
//! fn main() -> Result<(), rt::Error> {
//!     let mut runtime = Runtime::new()?;
//!     runtime.run_on_workers(|mut runtime_ref| -> Result<(), !> {
//!         // Spawn the actor.
//!         let new_actor = actor as fn(_) -> _;
//!         let actor_ref = runtime_ref.spawn_local(NoSupervisor, new_actor, (), ActorOptions::default());
//!
//!         // Now we can use the actor reference to send the actor a message.
//!         actor_ref.try_send("Hello world".to_owned()).unwrap();
//!
//!         Ok(())
//!     })?;
//!     runtime.start()
//! }
//!
//! /// Our actor.
//! async fn actor(mut ctx: actor::Context<String>) {
//!     if let Ok(msg) = ctx.receive_next().await {
//!         println!("got message: {}", msg);
//!     }
//! }
//! ```
//!
//! # Sharing actor references
//!
//! All actor references can be cloned, which is the easiest way to share them.
//!
//! The example below shows how an actor reference is cloned to send a message
//! to the same actor.
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::supervisor::NoSupervisor;
//! use heph::{actor, rt, ActorOptions, Runtime};
//!
//! fn main() -> Result<(), rt::Error> {
//!     let mut runtime = Runtime::new()?;
//!     runtime.run_on_workers(|mut runtime_ref| -> Result<(), !> {
//!         let new_actor = actor as fn(_) -> _;
//!         let actor_ref = runtime_ref.spawn_local(NoSupervisor, new_actor, (), ActorOptions::default());
//!
//!         // To create another actor reference we can simply clone the
//!         // first one.
//!         let second_actor_ref = actor_ref.clone();
//!
//!         // Now we can use both references to send a message.
//!         actor_ref.try_send("Hello world".to_owned()).unwrap();
//!         second_actor_ref.try_send("Bye world".to_owned()).unwrap();
//!
//!         Ok(())
//!     })?;
//!     runtime.start()
//! }
//!
//! /// Our actor.
//! async fn actor(mut ctx: actor::Context<String>) {
//!     if let Ok(msg) = ctx.receive_next().await {
//!         println!("First message: {}", msg);
//!     }
//!
//!     if let Ok(msg) = ctx.receive_next().await {
//!         println!("Second message: {}", msg);
//!     }
//! }
//! ```

use std::any::TypeId;
use std::convert::TryFrom;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::iter::FromIterator;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{self, Poll};

use inbox::Sender;

pub mod rpc;
#[doc(no_inline)]
pub use rpc::{Rpc, RpcError, RpcMessage, RpcResponse};

/// Actor reference.
///
/// An actor reference reference can be used to send messages to an actor, for
/// more details see the [module] documentation.
///
/// [module]: crate::actor_ref
pub struct ActorRef<M> {
    kind: ActorRefKind<M>,
}

enum ActorRefKind<M> {
    /// Reference to an actor running on the same node.
    Local(Sender<M>),
    /// Reference that attempts to map the message to a different type first.
    Mapped(Arc<dyn MappedActorRef<M>>),
}

// We know that the `Local` variant is `Send` and `Sync`. Since the `Mapped`
// variant is a boxed version of `Local` so is that variant. This makes the
// entire `ActorRefKind` `Send` and `Sync`, as long as `M` is `Send` (as we
// could be sending the message across thread bounds).
unsafe impl<M: Send> Send for ActorRefKind<M> {}
unsafe impl<M: Send> Sync for ActorRefKind<M> {}

// `ActorRefKind` is safe to move around independent of `M` as it's already heap
// allocated.
impl<M> Unpin for ActorRefKind<M> {}

impl<M> ActorRef<M> {
    /// Create a new `ActorRef` for an actor using `inbox_ref`.
    pub(crate) const fn local(sender: Sender<M>) -> ActorRef<M> {
        ActorRef {
            kind: ActorRefKind::Local(sender),
        }
    }

    /// Send a message to the actor.
    ///
    /// See [Sending messages] and [`ActorRef::try_send`] for more details.
    ///
    /// [Sending messages]: index.html#sending-messages
    ///
    /// # Notes
    ///
    /// Mapped actor references, see [`ActorRef::map`] and
    /// [`ActorRef::try_map`], require an allocation and might be expansive. If
    /// possible try [`ActorRef::try_send`] first, which does not require an
    /// allocation. Regular (i.e. non-mapped) actor references do not require an
    /// allocation.
    pub fn send<'r, 'fut, Msg>(&'r self, msg: Msg) -> SendValue<'r, 'fut, M>
    where
        'r: 'fut,
        Msg: Into<M>,
    {
        use ActorRefKind::*;
        let msg = msg.into();
        SendValue {
            kind: match &self.kind {
                Local(sender) => SendValueKind::Local(sender.send(msg)),
                Mapped(actor_ref) => SendValueKind::Mapped(actor_ref.mapped_send(msg)),
            },
        }
    }

    /// Attempt to send a message to the actor.
    ///
    /// Some types of actor references can detect errors in sending a message,
    /// however not all actor references can. This means that even if this
    /// methods returns `Ok` it does **not** mean that the message is guaranteed
    /// to be delivered to or handled by the actor.
    ///
    /// See [Sending messages] for more details.
    ///
    /// [Sending messages]: index.html#sending-messages
    pub fn try_send<Msg>(&self, msg: Msg) -> Result<(), SendError>
    where
        Msg: Into<M>,
    {
        use ActorRefKind::*;
        #[cfg(any(test, feature = "test"))]
        if crate::test::should_lose_msg() {
            log::debug!("dropping message on purpose");
            return Ok(());
        }

        let msg = msg.into();
        match &self.kind {
            Local(sender) => sender.try_send(msg).map_err(|_| SendError),
            Mapped(actor_ref) => actor_ref.try_mapped_send(msg),
        }
    }

    /// Make a Remote Procedure Call (RPC).
    ///
    /// This will send the `request` to the actor and returns a [`Rpc`]
    /// [`Future`] that will return a response (of type `Res`), or an error if
    /// the receiving actor didn't respond.
    ///
    /// See the [`rpc`] module for more details.
    ///
    /// [`Future`]: std::future::Future
    pub fn rpc<'r, 'fut, Req, Res>(&'r self, request: Req) -> Rpc<'r, 'fut, M, Res>
    where
        'r: 'fut,
        M: From<RpcMessage<Req, Res>>,
    {
        Rpc::new(self, request)
    }

    /// Change the message type of the actor reference.
    ///
    /// Before sending the message this will first change the message into a
    /// different type. This is useful when you need to send to different types
    /// of actors (using different message types) from a central location. For
    /// example in process signal handling, see [`RuntimeRef::receive_signals`].
    ///
    /// [`RuntimeRef::receive_signals`]: crate::RuntimeRef::receive_signals
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
        M: From<Msg> + 'static,
        Msg: 'static,
    {
        // There is a blanket implementation for `TryFrom` for `T: From` so we
        // can use the `TryFrom` knowning that it will never return an error.
        self.try_map()
    }

    /// Much like [`map`], but uses the [`TryFrom`] trait.
    ///
    /// This creates a new actor reference that attempts to map from one message
    /// type to another before sending. This is useful when you need to send to
    /// different types of actors from a central location.
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
        M: TryFrom<Msg> + 'static,
        Msg: 'static,
    {
        if TypeId::of::<ActorRef<M>>() == TypeId::of::<ActorRef<Msg>>() {
            // Safety: If `M` == `Msg`, then the following `transmute` is a
            // no-op and thus safe.
            unsafe { std::mem::transmute(self) }
        } else {
            ActorRef {
                kind: ActorRefKind::Mapped(Arc::new(self)),
            }
        }
    }

    /// Returns `true` if the actor to which this reference sends to is still
    /// connected.
    ///
    /// # Notes
    ///
    /// Even if this returns `true` it doesn't mean [`ActorRef::try_send`] will
    /// succeeded (even if the inbox isn't full). There is always a race
    /// condition between using this method and doing something based on it.
    ///
    /// This does provide one useful feature: once this returns `false` it will
    /// never return `true` again. This makes it useful in the use case where
    /// creating a message is expansive, which is wasted if the actor is no
    /// longer running. Thus this should only be used as optimisation to not do
    /// work.
    pub fn is_connected(&self) -> bool {
        use ActorRefKind::*;
        match &self.kind {
            Local(sender) => sender.is_connected(),
            Mapped(actor_ref) => actor_ref.is_connected(),
        }
    }

    /// Returns true if `self` and `other` send messages to the same actor.
    pub fn sends_to<Msg>(&self, other: &ActorRef<Msg>) -> bool {
        self.id() == other.id()
    }

    fn id(&self) -> inbox::Id {
        use ActorRefKind::*;
        match &self.kind {
            Local(sender) => sender.id(),
            Mapped(actor_ref) => actor_ref.id(),
        }
    }
}

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> ActorRef<M> {
        use ActorRefKind::*;
        ActorRef {
            kind: match &self.kind {
                Local(sender) => Local(sender.clone()),
                Mapped(actor_ref) => Mapped(actor_ref.clone()),
            },
        }
    }
}

impl<M> fmt::Debug for ActorRef<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ActorRef")
    }
}

/// Trait to erase the original message type of the actor reference.
///
/// # Notes
///
/// For correctness this may only be implemented on [`ActorRef`].
trait MappedActorRef<M> {
    /// Same as [`ActorRef::try_send`] but converts the message first.
    fn try_mapped_send(&self, msg: M) -> Result<(), SendError>;

    fn mapped_send<'r, 'fut>(
        &'r self,
        msg: M,
    ) -> Pin<Box<dyn Future<Output = Result<(), SendError>> + 'fut>>
    where
        'r: 'fut,
        M: 'fut;

    fn is_connected(&self) -> bool;

    fn id(&self) -> inbox::Id;
}

impl<M, Msg> MappedActorRef<Msg> for ActorRef<M>
where
    M: TryFrom<Msg>,
{
    fn try_mapped_send(&self, msg: Msg) -> Result<(), SendError> {
        M::try_from(msg)
            .map_err(|_msg| SendError)
            .and_then(|msg| self.try_send(msg))
    }

    fn mapped_send<'r, 'fut>(
        &'r self,
        msg: Msg,
    ) -> Pin<Box<dyn Future<Output = Result<(), SendError>> + 'fut>>
    where
        'r: 'fut,
        Msg: 'fut,
    {
        let mapped_send = match M::try_from(msg) {
            Ok(msg) => MappedSendValue::Send(self.send(msg)),
            Err(..) => MappedSendValue::MapErr,
        };
        Box::pin(mapped_send)
    }

    fn is_connected(&self) -> bool {
        self.is_connected()
    }

    fn id(&self) -> inbox::Id {
        self.id()
    }
}

enum MappedSendValue<'r, 'fut, M> {
    Send(SendValue<'r, 'fut, M>),
    /// Error mapping the message type.
    MapErr,
}

impl<'r, 'fut, M> Future for MappedSendValue<'r, 'fut, M> {
    type Output = Result<(), SendError>;

    #[track_caller]
    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        use MappedSendValue::*;
        // Safety: we're not moving the future to this is safe.
        let this = unsafe { self.get_unchecked_mut() };
        match this {
            // Safety: we're not moving `send_value` so this is safe.
            Send(send_value) => unsafe { Pin::new_unchecked(send_value) }
                .poll(ctx)
                .map_err(|_| SendError),
            MapErr => Poll::Ready(Err(SendError)),
        }
    }
}

/// [`Future`] behind [`ActorRef::send`].
///
/// # Safety
///
/// It is not safe to leak this `SendValue` (by using [`mem::forget`]). Always
/// make sure the destructor is run, by calling [`drop`], or letting it go out
/// of scope.
///
/// [`mem::forget`]: std::mem::forget
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct SendValue<'r, 'fut, M> {
    kind: SendValueKind<'r, 'fut, M>,
}

enum SendValueKind<'r, 'fut, M> {
    Local(inbox::SendValue<'r, M>),
    Mapped(Pin<Box<dyn Future<Output = Result<(), SendError>> + 'fut>>),
}

// We know that the `Local` variant is `Send` and `Sync`. Since the `Mapped`
// variant is a boxed version of `Local` so is that variant. This makes the
// entire `SendValueKind` `Send` and `Sync`, as long as `M` is `Send` (as we
// could be sending the message across thread bounds).
unsafe impl<'r, 'fut, M: Send> Send for SendValueKind<'r, 'fut, M> {}
unsafe impl<'r, 'fut, M: Send> Sync for SendValueKind<'r, 'fut, M> {}

impl<'r, 'fut, M> Future for SendValue<'r, 'fut, M> {
    type Output = Result<(), SendError>;

    #[track_caller]
    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        use SendValueKind::*;
        #[cfg(any(test, feature = "test"))]
        if crate::test::should_lose_msg() {
            log::debug!("dropping message on purpose");
            return Poll::Ready(Ok(()));
        }

        // Safety: we're not moving the future to this is safe.
        let this = unsafe { self.get_unchecked_mut() };
        match &mut this.kind {
            // Safety: we're not moving `inner` so this is safe.
            Local(send_value) => unsafe { Pin::new_unchecked(send_value) }
                .poll(ctx)
                .map_err(|_| SendError),
            Mapped(fut) => fut.as_mut().poll(ctx),
        }
    }
}

impl<'r, 'fut, M> fmt::Debug for SendValue<'r, 'fut, M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SendValue")
    }
}

/// Error returned when sending a message fails.
///
/// The reason why the sending of the message failed is unspecified.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SendError;

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unable to send message")
    }
}

impl Error for SendError {}

/// A group of [`ActorRef`]s used to send a message to multiple actors.
pub struct ActorGroup<M> {
    actor_refs: Vec<ActorRef<M>>,
    /// Index of the actor reference to send the [single delivery] message to.
    /// Using relaxed ordering on this field is fine because we make no
    /// gaurantees about to which actor the message will be delivered. E.g. we
    /// could always send to the first actor in the group and still forfill the
    /// contract.
    ///
    /// [single delivery]: Delivery::ToOne
    send_next: AtomicUsize,
}

/// The kind of delivery to use in [`ActorGroup::try_send`].
#[derive(Copy, Clone, Debug)]
pub enum Delivery {
    /// Delivery a copy of the message to all actors in the group.
    ToAll,
    /// Delivery the message to one of the actors.
    ToOne,
}

impl<M> ActorGroup<M> {
    /// Creates an empty `ActorGroup`.
    pub const fn empty() -> ActorGroup<M> {
        ActorGroup {
            actor_refs: Vec::new(),
            send_next: AtomicUsize::new(0),
        }
    }

    /// Create a new `ActorGroup`.
    pub fn new<I>(actor_refs: I) -> ActorGroup<M>
    where
        I: IntoIterator<Item = ActorRef<M>>,
    {
        ActorGroup {
            actor_refs: actor_refs.into_iter().collect(),
            send_next: AtomicUsize::new(0),
        }
    }

    /// Returns the number of actor references in the group.
    pub fn len(&self) -> usize {
        self.actor_refs.len()
    }

    /// Returns `true` if the group is empty.
    pub fn is_empty(&self) -> bool {
        self.actor_refs.is_empty()
    }

    /// Add an `ActorRef` to the group.
    pub fn add(&mut self, actor_ref: ActorRef<M>) {
        self.actor_refs.push(actor_ref)
    }

    /// Remove all actor references that have been disconnected.
    pub fn remove_disconnected(&mut self) {
        self.actor_refs.retain(ActorRef::is_connected);
    }

    /// Attempts to send a message to all the actors in the group.
    ///
    /// This can either send the message to a single actor, by using
    /// [`Delivery::ToOne`], or to all actors in the group by using
    /// [`Delivery::ToAll`].
    ///
    /// When deliverying to all actors this will first `clone` the message and
    /// then [`try_send`]ing it to each actor in the group. Note that this means
    /// it will `clone` before calling [`Into::into`] on the message. If the
    /// call to [`Into::into`] is expansive, or `M` is cheaper to clone than
    /// `Msg` it might be worthwhile to call `msg.into()` before calling this
    /// method.
    ///
    /// This only returns an error if the group is empty, otherwise this will
    /// always return `Ok(())`.
    ///
    /// See [Sending messages] for more details.
    ///
    /// [`try_send`]: ActorRef::try_send
    /// [Sending messages]: index.html#sending-messages
    pub fn try_send<Msg>(&self, msg: Msg, delivery: Delivery) -> Result<(), SendError>
    where
        Msg: Into<M> + Clone,
    {
        if self.actor_refs.is_empty() {
            return Err(SendError);
        }

        match delivery {
            Delivery::ToAll => {
                for actor_ref in &self.actor_refs {
                    let _ = actor_ref.try_send(msg.clone());
                }
                Ok(())
            }
            Delivery::ToOne => {
                // NOTE: this wraps around on overflow.
                let idx = self.send_next.fetch_add(1, Ordering::Relaxed) % self.actor_refs.len();
                let actor_ref = &self.actor_refs[idx];
                // TODO: try to send it to another actor on send failure?
                actor_ref.try_send(msg)
            }
        }
    }
}

impl<M> FromIterator<ActorRef<M>> for ActorGroup<M> {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = ActorRef<M>>,
    {
        ActorGroup::new(iter)
    }
}

impl<M> Extend<ActorRef<M>> for ActorGroup<M> {
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = ActorRef<M>>,
    {
        self.actor_refs.extend(iter);
    }
}

impl<M> fmt::Debug for ActorGroup<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.actor_refs.fmt(f)
    }
}
