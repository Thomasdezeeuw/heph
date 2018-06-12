//! Module containing the `ActorRef` and related types.

use std::marker::PhantomData;

use system::ActorId;
use system::error::SendError;

use actor::Actor;

// TODO: add examples on how to share the reference and how to send messages.

/// A reference to an actor inside a running [`ActorSystem`].
///
/// This reference can be used to send messages to the actor. To share this
/// reference simply clone it.
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorRef<A> {
    id: ActorId,
    message_type: PhantomData<A>,
}

impl<'a, A> ActorRef<A>
    where A: Actor<'a>,
{
    /// Get the `ActorId`.
    pub(super) fn id(&self) -> ActorId {
        self.id
    }

    /// Send a message to the actor.
    pub fn send<M>(&mut self, _msg: M) -> Result<(), SendError<M>>
        where M: Into<A::Message>,
    {
        unimplemented!("ActorRef.send");
    }
}

impl<A> Clone for ActorRef<A> {
    fn clone(&self) -> ActorRef<A> {
        ActorRef {
            id: self.id,
            message_type: PhantomData,
        }
    }
}
