use std::collections::VecDeque;
use std::marker::PhantomData;

use actor::Actor;
use system::ActorId;

/// A process that represent an actor, it's mailbox and current execution.
#[derive(Debug)]
pub struct Process<'a, A>
    where A: Actor<'a>,
{
    /// Unique id in the `ActorSystem`.
    id: ActorId,
    /// Inbox of the actor.
    inbox: VecDeque<A::Message>,
    /// The actor.
    actor: A,

    // TODO: add ActorOptions, e.g. scheduling Priority.
}

impl<'a, A> Process<'a, A>
    where A: Actor<'a>,
{
    /// Create a new process.
    pub(super) fn new(id: ActorId, actor: A) -> Process<'a, A> {
        Process {
            id,
            inbox: VecDeque::new(),
            actor,
        }
    }

    /// Deliver a new message to actor.
    pub fn deliver(&mut self, msg: A::Message) {
        self.inbox.push_back(msg)
    }

    /// Receive a delivered message, if any.
    pub fn receive(&mut self) -> Option<A::Message> {
        self.inbox.pop_front()
    }
}
