//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::io;
use std::cell::RefCell;
use std::rc::Rc;

use futures_core::Async;
use futures_core::task::LocalMap;
use mio_st::event::Ready;
use mio_st::poll::{Poll, PollOpt};
use mio_st::registration::Registration;

use actor::Actor;
use system::ActorSystemRef;
use system::options::ActorOptions;
use system::process::{Process, ProcessId, ProcessCompletion};
use system::scheduler::Priority;

mod actor_ref;
mod actor_waker;
mod mailbox;

pub use self::actor_ref::ActorRef;
pub use self::actor_waker::ActorProcessWaker;
pub use self::mailbox::MailBox;

/// The mailbox of an `Actor` that is shared between an `ActorProcess` and one
/// or more `ActorRef`s.
type SharedMailbox<M> = Rc<RefCell<MailBox<M>>>;

/// A process that represent an actor, it's mailbox and current execution.
#[derive(Debug)]
pub struct ActorProcess<A>
    where A: Actor,
{
    /// Unique id in the `ActorSystem`.
    id: ProcessId,
    /// The priority of the process.
    priority: Priority,
    /// The actor.
    actor: A,
    /// Whether or not the actor has returned `Async::Ready` and is ready to
    /// handle another message.
    ready_for_msg: bool,
    /// Map with local data for the futures task system.
    map: LocalMap,
    /// Notifier to the `registration`, used in waking the process.
    waker: ActorProcessWaker,
    /// Inbox of the actor.
    inbox: SharedMailbox<A::Message>,
    /// Registration needs to live as long as this process is alive.
    registration: Registration,
}

impl<A> ActorProcess<A>
    where A: Actor,
{
    /// Create a new process.
    pub fn new(id: ProcessId, actor: A, options: ActorOptions, poll: &mut Poll) -> Result<ActorProcess<A>, (A, io::Error)> {
        let (mut registration, notifier) = Registration::new();
        if let Err(err) = poll.register(&mut registration, id.into(), Ready::READABLE, PollOpt::Edge) {
            return Err((actor, err));
        }

        let inbox = Rc::new(RefCell::new(MailBox::new(notifier.clone())));

        Ok(ActorProcess {
            id,
            priority: options.priority,
            actor,
            ready_for_msg: true,
            map: LocalMap::new(),
            waker: ActorProcessWaker::new(notifier),
            inbox,
            registration,
        })
    }

    /// Create a new `ActorRef` reference to this actor.
    pub fn create_ref(&self) -> ActorRef<A> {
        ActorRef::new(Rc::clone(&self.inbox))
    }
}

impl<A> Process for ActorProcess<A>
    where A: Actor,
{
    fn id(&self) -> ProcessId {
        self.id
    }

    fn priority(&self) -> Priority {
        self.priority
    }

    fn run(&mut self, system_ref: &mut ActorSystemRef) -> ProcessCompletion {
        // Create our future execution context.
        let waker = self.waker.waker();
        let mut ctx = system_ref.create_context(&mut self.map, &waker);

        loop {
            // First handle the message the actor is currently handling, if any.
            if !self.ready_for_msg {
                match self.actor.poll(&mut ctx) {
                    Ok(Async::Ready(())) => {
                        self.ready_for_msg = true;
                        // Continue to handle another message.
                    },
                    Ok(Async::Pending) => return ProcessCompletion::Pending,
                    Err(_err) => {
                        // TODO: send error to supervisor.
                        return ProcessCompletion::Complete
                    },
                }
            }

            if let Some(msg) = self.inbox.borrow_mut().receive() {
                self.ready_for_msg = false;
                match self.actor.handle(&mut ctx, msg) {
                    Ok(Async::Ready(())) => {
                        self.ready_for_msg = true;
                        // Try to handle another message.
                        continue
                    },
                    Ok(Async::Pending) => return ProcessCompletion::Pending,
                    Err(_err) => {
                        // TODO: send error to supervisor.
                        return ProcessCompletion::Complete
                    },
                }
            } else {
                return ProcessCompletion::Pending
            }
        }
    }
}
