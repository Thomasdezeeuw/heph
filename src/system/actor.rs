//! TODO: docs.

use std::marker::PhantomData;

use actor::Actor;

/// Unique id for each actor.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct ActorId(u64);

/// Options for add an actor to the system.
#[derive(Debug)]
pub struct ActorOptions {
    /// Priority for the actor in scheduling queue.
    pub priority: Priority,
    _priv: (),
}

/// Priority for the actor in scheduling queue.
///
/// Lower is better.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Priority(u8);

/// Lowest priority possible priority.
pub const PRIORITY_MIN: u8 = 19;

// TODO: impl Ord and PartialOrd for `Priority`.

impl Priority {
    /// Create a new priority.
    ///
    /// # Panics
    ///
    /// This function panics if the `priority` value is higher then
    /// [`PRIORITY_MIN`].
    ///
    /// [`PRIORITY_MIN`]: const.PRIORITY_MIN.html
    fn new(priority: u8) -> Priority {
        if priority > PRIORITY_MIN {
            panic!("priority `{}` is invalid, it must be between 0..{}", priority, PRIORITY_MIN);
        }
        Priority(priority)
    }
}

/// A reference to an actor.
///
/// This reference can be used to send messages to it. To share this reference
/// simply clone it.
// TODO: add example on how to share the reference and how to send messages.
#[derive(Debug)]
pub struct ActorRef<A> {
    id: ActorId,
    message_type: PhantomData<A>,
}

// TODO: implement Clone.

impl<'a, A> ActorRef<A>
    where A: Actor<'a>,
{
    /// Get the `ActorId`.
    pub(crate) fn id(&self) -> ActorId {
        self.id
    }

    /// Send a message to the actor.
    pub fn send<M>(&mut self, _msg: M) -> SendError<M>
        where M: Into<A::Message>,
    {
        unimplemented!("ActorRef.send");
    }
}

/// Error when sending messages goes wrong.
#[derive(Debug, Eq, PartialEq)]
pub struct SendError<M> {
    /// The message that failed to send.
    pub message: M,
    /// The reason why the sending failed.
    pub reason: SendErrorReason,
}

/// The reason why sending a message failed.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum SendErrorReason {
    /// The actor to which the message was meant to be sent is shutdown.
    ActorShutdown,
    /// The system is shutting down.
    ///
    /// When the system is shutting down no more message sending is allowed.
    SystemShutdown,
}
