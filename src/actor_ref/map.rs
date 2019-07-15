//! Module containing the mapped actor reference types.

use std::{fmt, marker};

use crate::actor_ref::{Send, SendError};

// TODO: remove the need for a allocation for both `LocalMap` and `Map`, if that
// is possible at all.

/// Trait to erase the original message type of the actor reference.
pub(super) trait MappedSend<Msg> {
    fn mapped_send(&mut self, msg: Msg) -> Result<(), SendError<Msg>>;
}

impl<T, M, Msg> MappedSend<Msg> for T
where
    T: Send<Message = M>,
    Msg: Into<M> + From<M>,
{
    fn mapped_send(&mut self, msg: Msg) -> Result<(), SendError<Msg>> {
        self.send(msg.into()).map_err(|err| SendError {
            message: Msg::from(err.message),
        })
    }
}

/// Actor reference that maps from one type to another.
///
/// This reference wraps another actor reference and changes the message type.
/// This is useful when you need to send to different types of actors from a
/// central location. See [`ActorRef::local_map`].
///
/// This actor reference doesn't implement [`Send`] or [`Sync`], see [`Map`] for
/// a version that does.
///
/// [`ActorRef::local_map`]: crate::actor_ref::ActorRef::local_map
/// [`Send`]: std::marker::Send
/// [`Sync`]: std::marker::Sync
pub struct LocalMap<M> {
    pub(super) inner: Box<dyn MappedSend<M>>,
}

impl<M> Send for LocalMap<M> {
    type Message = M;

    fn send(&mut self, msg: Self::Message) -> Result<(), SendError<Self::Message>> {
        self.inner.mapped_send(msg)
    }
}

impl<M> fmt::Debug for LocalMap<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalMappedActorRef")
    }
}

/// Version of [`LocalMap`] that is thread safe, i.e. implements [`Send`] +
/// [`Sync`].
///
/// See [`ActorRef::map`].
///
/// [`Send`]: std::marker::Send
/// [`Sync`]: std::marker::Sync
/// [`ActorRef::map`]: crate::actor_ref::ActorRef::map
pub struct Map<M> {
    pub(super) inner: Box<dyn MappedSend<M> + marker::Send + Sync>,
}

impl<M> Send for Map<M> {
    type Message = M;

    fn send(&mut self, msg: Self::Message) -> Result<(), SendError<Self::Message>> {
        self.inner.mapped_send(msg)
    }
}

impl<M> fmt::Debug for Map<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MappedActorRef")
    }
}
