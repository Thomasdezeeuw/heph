//! Module containing actor references.
//!
//! An actor reference is a generic reference to an actor that can run on the
//! same thread, another thread on the same machine or even running remotely.
//!
//! Currently there are two types of actor references.
//!
//! - [`Local`] (`ActorRef<M, Local>`): reference to an actor running on the
//!   same thread. This is the least expensive reference and should always be
//!   preferred, that is why it is also the default.
//! - [`Machine`] (`ActorRef<M, Machine>`): reference to an actor running on the
//!   same machine, possibly on another thread. This implements [`Send`] and
//!   [`Sync`], which the local actor reference does not.
//!
//! [`Local`]: crate::actor_ref::Local
//! [`Machine`]: crate::actor_ref::Machine
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
//! #![feature(async_await, await_macro, futures_api, never_type)]
//!
//! use heph::actor::Context;
//! use heph::supervisor::NoSupervisor;
//! use heph::system::{ActorOptions, ActorSystem, RuntimeError};
//!
//! fn main() -> Result<(), RuntimeError> {
//!     ActorSystem::new().with_setup(|mut system_ref| {
//!         // Add the actor to the actor system.
//!         let new_actor = actor as fn (_) -> _;
//!         let mut actor_ref = system_ref.spawn(NoSupervisor, new_actor, (), ActorOptions::default());
//!
//!         // Now we can use the reference to send the actor a message.
//!         actor_ref <<= "Hello world".to_owned();
//!         // Above is the same as:
//!         // let _ = actor_ref.send("Hello world".to_owned());
//!
//!         Ok(())
//!     })
//!     .run()
//! }
//!
//! /// Our actor.
//! async fn actor(mut ctx: Context<String>) -> Result<(), !> {
//!     let msg = await!(ctx.receive_next());
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
//! #![feature(async_await, await_macro, futures_api, never_type)]
//!
//! use heph::actor::Context;
//! use heph::supervisor::NoSupervisor;
//! use heph::system::{ActorOptions, ActorSystem, RuntimeError};
//!
//! fn main() -> Result<(), RuntimeError> {
//!      ActorSystem::new().with_setup(|mut system_ref| {
//!         let new_actor = actor as fn (_) -> _;
//!         let mut actor_ref = system_ref.spawn(NoSupervisor, new_actor, (), ActorOptions::default());
//!
//!         // To create another actor reference we can simply clone the first one.
//!         let mut second_actor_ref = actor_ref.clone();
//!
//!         // Now we can use both references to send a message.
//!         actor_ref <<= "Hello world".to_owned();
//!         second_actor_ref <<= "Bye world".to_owned();
//!
//!         Ok(())
//!     })
//!     .run()
//! }
//!
//! /// Our actor.
//! async fn actor(mut ctx: Context<String>) -> Result<(), !> {
//!     let msg = await!(ctx.receive_next());
//!     println!("First message: {}", msg);
//!
//!     let msg = await!(ctx.receive_next());
//!     println!("Second message: {}", msg);
//!     Ok(())
//! }
//! ```

use std::fmt;
use std::ops::ShlAssign;

use crate::system::ActorSystemRef;

mod error;

#[cfg(all(test, feature = "test"))]
mod tests;

pub mod local;
pub mod machine;

pub use self::error::{ActorShutdown, SendError};
pub use self::local::Local;
pub use self::machine::Machine;

/// A reference to an actor.
///
/// This reference can be used to send messages to an actor, for more details
/// see the [module] documentation.
///
/// `ActorRef` is effectively just a container that wraps the actual
/// implementation defined by the [`ActorRefType`].
///
/// [module]: crate::actor_ref
#[repr(transparent)]
pub struct ActorRef<M, T: ActorRefType<M> = Local> {
    data: T::Data,
}

/// Trait that defines the type of actor reference.
///
/// This trait allows for different types of actor reference to use the same
/// `ActorRef` struct, which allows for easier usage.
pub trait ActorRefType<M> {
    /// Data required by the actor reference.
    type Data: Sized;

    /// Implementation behind [`ActorRef::send`].
    fn send(data: &mut Self::Data, msg: M) -> Result<(), SendError<M>>;
}

impl<M, T> ActorRef<M, T>
    where T: ActorRefType<M>,
{
    /// Asynchronously send a message to the actor.
    ///
    /// Some types of actor references can detect errors in sending a message,
    /// however not all actor references can. This means that even if this
    /// methods returns `Ok` it does **not** means that the message is
    /// guaranteed to be handled by the actor.
    ///
    /// See [Sending messages] for more details.
    ///
    /// [Sending messages]: index.html#sending-messages
    pub fn send<Msg>(&mut self, msg: Msg) -> Result<(), SendError<M>>
        where Msg: Into<M>,
    {
        T::send(&mut self.data, msg.into())
    }
}

impl<M> ActorRef<M, Local> {
    /// Upgrade a local actor reference to a machine local reference.
    ///
    /// This allows the actor reference to be send across threads, however
    /// operations on it are more expensive.
    pub fn upgrade(self, system_ref: &mut ActorSystemRef) -> Result<ActorRef<M, Machine>, ActorShutdown> {
        let (pid, sender) = match self.data.inbox.upgrade() {
            Some(mut inbox) => inbox.borrow_mut().upgrade_ref(),
            None => return Err(ActorShutdown),
        };

        let waker = system_ref.new_waker(pid);
        Ok(ActorRef::<M, Machine>::new(sender, waker.into()))
    }
}

impl<M, Msg, T> ShlAssign<Msg> for ActorRef<M, T>
    where T: ActorRefType<M>,
          Msg: Into<M>,
{
    fn shl_assign(&mut self, msg: Msg) {
        let _ = self.send(msg);
    }
}

impl<M, T> Clone for ActorRef<M, T>
    where T: ActorRefType<M>,
          T::Data: Clone,
{
    fn clone(&self) -> ActorRef<M, T> {
        ActorRef {
            data: self.data.clone(),
        }
    }
}

impl<M, T> Eq for ActorRef<M, T>
    where T: ActorRefType<M>,
          T::Data: Eq,
{}

impl<M, T> PartialEq for ActorRef<M, T>
    where T: ActorRefType<M>,
          T::Data: PartialEq,
{
    fn eq(&self, other: &ActorRef<M, T>) -> bool {
        self.data.eq(&other.data)
    }
}

impl<M, T> fmt::Debug for ActorRef<M, T>
    where T: ActorRefType<M>,
          T::Data: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.data.fmt(f)
    }
}
