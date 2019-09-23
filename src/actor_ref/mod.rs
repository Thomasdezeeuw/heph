//! Module containing actor references.
//!
//! An actor reference is a generic reference to an actor that can run on the
//! same thread, another thread on the same machine or even running remotely.
//!
//! All available actor reference types can be found in the [`types`] module,
//! the three main types of actor references are the following:
//!
//! - [`Local`] (`ActorRef<Local<M>>`): reference to an actor running on the
//!   same thread. This is the least expensive reference and should always be
//!   preferred, that is why it is also the default.
//! - [`Machine`] (`ActorRef<Machine<M>>`): reference to an actor running on the
//!   same machine, possibly on another thread. This implements
//!   [`Send`](std::marker::Send) and [`Sync`](std::marker::Sync), which the
//!   local actor reference does not.
//! - [`Sync`] (`ActorRef<Sync<M>>`): reference to a synchronous actor running
//!   on its own thread. Like the machine reference this reference also
//!   implements [`Send`](std::marker::Send) and [`Sync`](std::marker::Sync).
//!
//! [`types`]: crate::actor_ref::types
//! [`Local`]: crate::actor_ref::Local
//! [`Machine`]: crate::actor_ref::Machine
//! [`Sync`]: crate::actor_ref::Sync
//!
//! ## Sending messages
//!
//! All types of actor references have a [`send`] method. These methods don't
//! block, even on the remote actor reference, but the method doesn't provided a
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
//! received and/or processed correctly. This can for example be done by using
//! the [request-response pattern].
//!
//! Other then the [`send`] method the `<<=` operator can be used to send
//! messages, which does the same thing as `send` but with nicer syntax. The
//! following example shows how messages can be send using this operator. It
//! uses a local actor reference but it's the same for all flavours.
//!
//! [request-response pattern]: ../channel/oneshot/index.html#request-response-pattern
//! [`send`]: crate::actor_ref::ActorRef::send
//!
//! ```
//! #![feature(never_type)]
//!
//! use heph::supervisor::NoSupervisor;
//! use heph::system::RuntimeError;
//! use heph::{actor, ActorOptions, ActorSystem};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     ActorSystem::new()
//!         .with_setup(|mut system_ref| {
//!             // Add the actor to the actor system.
//!             let new_actor = actor as fn(_) -> _;
//!             let mut actor_ref = system_ref.spawn(NoSupervisor, new_actor, (), ActorOptions::default());
//!
//!             // Now we can use the reference to send the actor a message.
//!             actor_ref <<= "Hello world".to_owned();
//!             // Above is the same as:
//!             // let _ = actor_ref.send("Hello world".to_owned());
//!
//!             Ok(())
//!         })
//!         .run()
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
//! use heph::system::RuntimeError;
//! use heph::{actor, ActorOptions, ActorSystem};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     ActorSystem::new()
//!         .with_setup(|mut system_ref| {
//!             let new_actor = actor as fn(_) -> _;
//!             let mut actor_ref = system_ref.spawn(NoSupervisor, new_actor, (), ActorOptions::default());
//!
//!             // To create another actor reference we can simply clone the first one.
//!             let mut second_actor_ref = actor_ref.clone();
//!
//!             // Now we can use both references to send a message.
//!             actor_ref <<= "Hello world".to_owned();
//!             second_actor_ref <<= "Bye world".to_owned();
//!
//!             Ok(())
//!         })
//!         .run()
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
use std::ops::ShlAssign;
use std::{fmt, marker};

use crate::system::ActorSystemRef;

mod error;

#[cfg(test)]
mod tests;

mod local;
mod machine;
mod map;
mod sync;

pub mod types {
    //! Actor reference types.

    pub use super::local::Local;
    pub use super::machine::Machine;
    pub use super::map::{LocalMap, LocalTryMap, Map, TryMap};
    pub use super::sync::Sync;
}

pub use error::{ActorShutdown, SendError};
#[doc(no_inline)]
pub use types::{Local, LocalMap, LocalTryMap, Machine, Map, Sync, TryMap};

/// A reference to an actor.
///
/// This reference can be used to send messages to an actor, for more details
/// see the [module] documentation.
///
/// `ActorRef` is effectively just a container that wraps the actual
/// implementation defined by the [`Send`].
///
/// [module]: crate::actor_ref
#[repr(transparent)]
#[derive(Clone, Eq, PartialEq)]
pub struct ActorRef<T> {
    inner: T,
}

impl<T> ActorRef<T> {
    /// Create a new `ActorRef`.
    pub(crate) const fn new(inner: T) -> ActorRef<T> {
        ActorRef { inner }
    }
}

/// Trait that defines how an actor reference sends messages.
pub trait Send {
    /// The message the actor reference can send.
    type Message;

    /// Implementation behind [`ActorRef::send`].
    fn send(&mut self, msg: Self::Message) -> Result<(), SendError<Self::Message>>;
}

impl<M, T> ActorRef<T>
where
    T: Send<Message = M>,
{
    /// Asynchronously send a message to the actor.
    ///
    /// Some types of actor references can detect errors in sending a message,
    /// however not all actor references can. This means that even if this
    /// methods returns `Ok` it does **not** mean that the message is
    /// guaranteed to be handled by the actor.
    ///
    /// See [Sending messages] for more details.
    ///
    /// [Sending messages]: index.html#sending-messages
    pub fn send<Msg>(&mut self, msg: Msg) -> Result<(), SendError<M>>
    where
        Msg: Into<M>,
    {
        self.inner.send(msg.into())
    }

    /// Wraps the actor reference to change the message type. This is useful
    /// when you need to send to different types of actors from a central
    /// location.
    ///
    /// # Notes
    ///
    /// This conversion is **not** cheap, it requires an allocation so use with
    /// caution when it comes to performance sensitive code.
    pub fn local_map<Msg>(self) -> ActorRef<LocalMap<Msg>>
    where
        T: 'static,
        M: From<Msg> + Into<Msg>,
    {
        ActorRef::new(LocalMap {
            inner: Box::new(self.inner),
        })
    }

    /// Same as [`local_map`] but returns a thread safe version.
    ///
    /// [`local_map`]: ActorRef::local_map
    ///
    /// # Notes
    ///
    /// This conversion is **not** cheap, it requires an allocation so use with
    /// caution when it comes to performance sensitive code.
    pub fn map<Msg>(self) -> ActorRef<Map<Msg>>
    where
        T: Send<Message = M> + marker::Sync + marker::Send + 'static,
        M: From<Msg> + Into<Msg>,
    {
        ActorRef::new(Map {
            inner: Box::new(self.inner),
        })
    }

    /// Much like [`local_map`], but uses the [`TryFrom`] trait.
    ///
    /// [`local_map`]: ActorRef::local_map
    ///
    /// # Notes
    ///
    /// This conversion is **not** cheap, it requires an allocation so use with
    /// caution when it comes to performance sensitive code.
    pub fn local_try_map<Msg>(self) -> ActorRef<LocalTryMap<Msg>>
    where
        T: 'static,
        M: TryFrom<Msg, Error = Msg> + Into<Msg>,
    {
        ActorRef::new(LocalTryMap {
            inner: Box::new(self.inner),
        })
    }

    /// Same as [`local_try_map`] but returns a thread safe version.
    ///
    /// [`local_try_map`]: ActorRef::local_try_map
    ///
    /// # Notes
    ///
    /// This conversion is **not** cheap, it requires an allocation so use with
    /// caution when it comes to performance sensitive code.
    pub fn try_map<Msg>(self) -> ActorRef<TryMap<Msg>>
    where
        T: Send<Message = M> + marker::Sync + marker::Send + 'static,
        M: TryFrom<Msg, Error = Msg> + Into<Msg>,
    {
        ActorRef::new(TryMap {
            inner: Box::new(self.inner),
        })
    }
}

impl<M> ActorRef<Local<M>> {
    /// Upgrade a local actor reference to a machine local reference.
    ///
    /// This allows the actor reference to be send across threads, however
    /// operations on it are more expensive.
    pub fn upgrade(
        mut self,
        system_ref: &mut ActorSystemRef,
    ) -> Result<ActorRef<Machine<M>>, ActorShutdown> {
        match self.inner.inbox.try_upgrade_ref() {
            Ok((pid, sender)) => {
                let waker = system_ref.new_waker(pid);
                Ok(ActorRef::new_machine(sender, waker))
            }
            Err(()) => Err(ActorShutdown),
        }
    }
}

impl<M, Msg, T> ShlAssign<Msg> for ActorRef<T>
where
    T: Send<Message = M>,
    Msg: Into<M>,
{
    fn shl_assign(&mut self, msg: Msg) {
        let _ = self.send(msg);
    }
}

impl<T> fmt::Debug for ActorRef<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}
