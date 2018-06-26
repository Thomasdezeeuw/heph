//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::{fmt, io};
use std::cell::RefCell;
use std::rc::Rc;
use std::task::Poll;

use mio_st::event::Ready;
use mio_st::poll::{Poll as MioPoll, PollOpt};
use mio_st::registration::Registration;

use actor::{Actor, ActorContext, ActorResult, Status};
use system::ActorSystemRef;
use system::options::ActorOptions;
use system::scheduler::process::{Process, ProcessId, ProcessCompletion};

mod actor_ref;
mod mailbox;

pub use self::actor_ref::ActorRef;
pub use self::mailbox::MailBox;

/// The mailbox of an `Actor` that is shared between an `ActorProcess` and one
/// or more `ActorRef`s.
type SharedMailbox<M> = Rc<RefCell<MailBox<M>>>;

/// A process that represent an actor, it's mailbox and current execution.
pub struct ActorProcess<A>
    where A: Actor,
{
    /// The actor.
    actor: A,
    /// Whether or not the actor has returned `Async::Ready` and is ready to
    /// handle another message.
    ready_for_msg: bool,
    /// Inbox of the actor.
    inbox: SharedMailbox<A::Message>,
    /// Registration needs to live as long as this process is alive.
    registration: Registration,
}

impl<A> ActorProcess<A>
    where A: Actor,
{
    /// Create a new process.
    pub fn new(pid: ProcessId, actor: A, _options: ActorOptions, poll: &mut MioPoll) -> Result<ActorProcess<A>, (A, io::Error)> {
        let (mut registration, notifier) = Registration::new();
        if let Err(err) = poll.register(&mut registration, pid.into(), Ready::READABLE, PollOpt::Edge) {
            return Err((actor, err));
        }

        let inbox = Rc::new(RefCell::new(MailBox::new(notifier.clone())));

        Ok(ActorProcess {
            actor,
            ready_for_msg: true,
            inbox,
            registration,
        })
    }

    /// Create a new reference to this actor.
    pub fn create_ref(&self) -> ActorRef<A> {
        ActorRef::new(Rc::downgrade(&self.inbox))
    }
}

impl<A> Process for ActorProcess<A>
    where A: Actor,
{
    fn run(&mut self, _system_ref: &mut ActorSystemRef) -> ProcessCompletion {
        // Create our actor execution context.
        let mut ctx = ActorContext{};

        loop {
            // First handle the message the actor is currently handling, if any.
            if !self.ready_for_msg {
                let res = self.actor.poll(&mut ctx);
                if let Some(status) = check_result(res) {
                    self.ready_for_msg = false;
                    return status;
                }
            }

            // Retrieve another message, if any.
            let msg = match self.inbox.borrow_mut().receive() {
                Some(msg) => msg,
                None => return ProcessCompletion::Pending,
            };

            // And pass the message to the actor.
            let res = self.actor.handle(&mut ctx, msg);
            if let Some(status) = check_result(res) {
                self.ready_for_msg = false;
                return status;
            }
        }
    }
}

impl<A> fmt::Debug for ActorProcess<A>
    where A: Actor,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: add fields.
        f.debug_struct("ActorProcess")
            .finish()
    }
}

/// Check the result of a call to poll or handle of an actor.
fn check_result<E>(result: ActorResult<E>) -> Option<ProcessCompletion> {
    match result {
        Poll::Ready(Ok(Status::Complete)) => Some(ProcessCompletion::Complete),
        Poll::Ready(Ok(Status::Ready)) => None,
        // TODO: send error to supervisor.
        Poll::Ready(Err(_err)) => Some(ProcessCompletion::Complete),
        Poll::Pending => Some(ProcessCompletion::Pending),
    }
}
