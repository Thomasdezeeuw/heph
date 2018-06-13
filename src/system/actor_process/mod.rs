//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::io;
use std::cell::RefCell;
use std::rc::Rc;

use mio_st::event::Ready;
use mio_st::poll::{Poll, PollOpt};
use mio_st::registration::Registration;

use actor::Actor;
use system::options::ActorOptions;
use system::scheduler::Priority;
use system::process::{Process, ProcessId, ProcessCompletion};

mod mailbox;
mod actor_ref;

pub use self::mailbox::MailBox;
pub use self::actor_ref::ActorRef;

/// The mailbox of an `Actor` that is shared between an `ActorProcess` and one
/// or more `ActorRef`s.
type SharedMailbox<M> = Rc<RefCell<MailBox<M>>>;

/// A process that represent an actor, it's mailbox and current execution.
#[derive(Debug)]
pub struct ActorProcess<'a, A>
    where A: Actor<'a>,
{
    /// Unique id in the `ActorSystem`.
    id: ProcessId,
    /// The priority of the process.
    priority: Priority,
    /// The actor.
    actor: A,
    /// Inbox of the actor.
    inbox: SharedMailbox<A::Message>,
    /// Registration needs to live as long as this process is alive.
    registration: Registration,
}

impl<'a, A> ActorProcess<'a, A>
    where A: Actor<'a>,
{
    /// Create a new process.
    pub fn new(id: ProcessId, actor: A, options: ActorOptions, poll: &mut Poll) -> io::Result<ActorProcess<'a, A>> {
        let (mut registration, notifier) = Registration::new();
        poll.register(&mut registration, id.into(), Ready::READABLE, PollOpt::Edge)?;

        let inbox = Rc::new(RefCell::new(MailBox::new(notifier)));

        Ok(ActorProcess {
            id,
            priority: options.priority,
            actor,
            inbox,
            registration,
        })
    }

    /// Create a new `ActorRef` reference to this actor.
    pub fn create_ref(&self) -> ActorRef<A::Message> {
        ActorRef::new(Rc::clone(&self.inbox))
    }
}

impl<'a, A> Process for ActorProcess<'a, A>
    where A: Actor<'a> + 'a,
{
    fn id(&self) -> ProcessId {
        self.id
    }

    fn priority(&self) -> Priority {
        self.priority
    }

    fn run(&mut self) -> ProcessCompletion {
        unimplemented!("ActorProcess.run");
    }
}
