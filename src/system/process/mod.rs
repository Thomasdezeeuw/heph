use std::collections::VecDeque;
use std::marker::PhantomData;

use actor::Actor;
use system::ActorId;
use system::scheduler::Priority;

/// The trait that represents a process for the `ActorSystem`.
pub trait Process {
    // TODO: provided a way to create a futures::task::Context, maybe by
    // providing an `ActorSystemRef`?

    /// Run the process.
    ///
    /// If this function returns it is assumed that the process is:
    /// - done completely, i.e. it doesn't have to be run anymore, or
    /// - would block, and it made sure it's scheduled at a later point.
    fn run(&mut self);

    /// Get the priority of the process.
    ///
    /// Used in scheduling the process.
    fn priority(&self) -> Priority;
}

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
