//! Module containing actor references.
//!
//! Actor references come in three flavours:
//! - [`LocalActorRef`]: reference to an actor running on the same thread,
//! - [`MachineLocalActorRef`]: reference to an actor running on the same
//!   machine, possibly on another thread, and
//! - [`RemoteActorRef`]: reference to an actor running on a different machine.
//!
//! These three flavours are combined into an more generic [`ActorRef`] type.
//!
//! [`LocalActorRef`]: struct.LocalActorRef.html
//! [`MachineLocalActorRef`]: struct.MachineLocalActorRef.html
//! [`RemoteActorRef`]: struct.RemoteActorRef.html
//! [`ActorRef`]: enum.ActorRef.html
//!
//! ## Sending messages
//!
//! All flavours of actor references have a `send` method. These methods don't
//! block, even on the remote actor reference, but the method doesn't provided a
//! lot of guarantees. What `send` does is add the message to the queue of
//! messages for the actor, asynchronously.
//!
//! In case of the local actor reference this can be done directly. But for
//! machine local actor references the message must first be send across thread
//! bounds before being added to the actor's message queue. Remote actor
//! references even need to send this message across a network, a lot can go
//! wrong here.
//!
//! This means that even if `send` returns `Ok` it doesn't mean the message is
//! received and handled by the actor. It could be that a remote actor is no
//! longer available, or that even a local actor crashes before the message is
//! handled.
//!
//! If guarantees are needed that a message is handled the receiving actor
//! should send back an acknowledgment that the message is received and handled
//! correctly.
//!
//! The following example shows how messages can be send. It uses a
//! `LocalActorRef` but it's the same for all flavours.
//!
//! ```
//! #![feature(async_await, await_macro, futures_api, never_type)]
//!
//! use heph::actor::{actor_factory, ActorContext};
//! use heph::supervisor::NoopSupervisor;
//! use heph::system::{ActorOptions, ActorSystem};
//!
//! /// Our actor.
//! async fn actor(mut ctx: ActorContext<String>, _: ()) -> Result<(), !> {
//!     let msg = await!(ctx.receive());
//!     println!("got message: {}", msg);
//!     Ok(())
//! }
//!
//! ActorSystem::new()
//!     .with_setup(|mut system_ref| {
//!         // Add the actor to the actor system.
//!         let new_actor = actor_factory(actor);
//!         let mut actor_ref = system_ref.spawn(NoopSupervisor, new_actor, (), ActorOptions::default());
//!
//!         // Now we can use the reference to send the actor a message.
//!         actor_ref.send("Hello world".to_owned());
//!         Ok(())
//!     })
//!     .run()
//!     .unwrap();
//! ```
//!
//! ## Sharing actor references
//!
//! All actor references can be cloned to be shared.
//!
//! The example below shows how an `LocalActorRef` is cloned to send a message
//! to the same actor.
//!
//! ```
//! #![feature(async_await, await_macro, futures_api, never_type)]
//!
//! use heph::actor::{actor_factory, ActorContext};
//! use heph::supervisor::NoopSupervisor;
//! use heph::system::{ActorOptions, ActorSystem};
//!
//! /// Our actor.
//! async fn actor(mut ctx: ActorContext<String>, _: ()) -> Result<(), !> {
//!     let msg = await!(ctx.receive());
//!     println!("got first message: {}", msg);
//!
//!     let msg = await!(ctx.receive());
//!     println!("got second message: {}", msg);
//!     Ok(())
//! }
//!
//! ActorSystem::new()
//!     .with_setup(|mut system_ref| {
//!         let new_actor = actor_factory(actor);
//!         let mut actor_ref = system_ref.spawn(NoopSupervisor, new_actor, (), ActorOptions::default());
//!
//!         // To create another `ActorRef` we can simply clone the first one.
//!         let mut second_actor_ref = actor_ref.clone();
//!
//!         // Now we can use both references to send a messsage.
//!         actor_ref.send("Hello world".to_owned())?;
//!         second_actor_ref.send("Byte world".to_owned())?;
//!         Ok(())
//!     })
//!     .run()
//!     .unwrap();
//! ```

use std::fmt;

use crate::error::SendError;

mod local;
mod machine;
mod remote;

#[cfg(all(test, feature = "test"))]
mod tests;

pub use self::local::LocalActorRef;
pub use self::machine::MachineLocalActorRef;
pub use self::remote::RemoteActorRef;

/// A reference to an actor.
///
/// This reference can be used to send messages to the actor running on the same
/// thread, on another thread or even on another machine.
///
/// This `ActorRef` can be created by using the `From` implementation on one of
/// the flavours of actor reference.
pub enum ActorRef<M> {
    /// A reference to a local actor, running on the same thread.
    Local(LocalActorRef<M>),
    /// A reference to an actor running on the same machine.
    Machine(MachineLocalActorRef<M>),
    /// A reference to a remote actor, running on a different machine.
    Remote(RemoteActorRef<M>),
}

impl<M> ActorRef<M> {
    /// Send a message to the actor.
    pub fn send<Msg>(&mut self, msg: Msg) -> Result<(), SendError<Msg>>
        where Msg: Into<M>,
    {
        use self::ActorRef::*;
        match self {
            Local(ref mut actor_ref) => actor_ref.send(msg),
            Machine(ref mut actor_ref) => { actor_ref.send(msg); Ok(()) },
            Remote(ref mut actor_ref) => actor_ref.send(msg),
        }
    }
}

impl<M> From<LocalActorRef<M>> for ActorRef<M> {
    fn from(actor_ref: LocalActorRef<M>) -> ActorRef<M> {
        ActorRef::Local(actor_ref)
    }
}

impl<M> From<MachineLocalActorRef<M>> for ActorRef<M> {
    fn from(actor_ref: MachineLocalActorRef<M>) -> ActorRef<M> {
        ActorRef::Machine(actor_ref)
    }
}

impl<M> From<RemoteActorRef<M>> for ActorRef<M> {
    fn from(actor_ref: RemoteActorRef<M>) -> ActorRef<M> {
        ActorRef::Remote(actor_ref)
    }
}

impl<M> fmt::Debug for ActorRef<M> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::ActorRef::*;
        f.debug_tuple("ActorRef")
            .field(match self {
                Local(ref actor_ref) => actor_ref,
                Machine(ref actor_ref) => actor_ref,
                Remote(ref actor_ref) => actor_ref,
            })
            .finish()
    }
}
