//! Module containing the implementation of the `Process` trait for `Actor`s.

use std::fmt;
use std::cell::RefCell;
use std::rc::Rc;
use std::task::Poll;

use actor::{Actor, ActorContext, Status};
use process::{Process, ProcessId, ProcessResult};
use system::ActorSystemRef;
use system::options::ActorOptions;

mod actor_ref;
mod mailbox;

pub use self::actor_ref::ActorRef;

use self::mailbox::MailBox;

/// A process that represent an actor, it's mailbox and current execution.
pub struct ActorProcess<A>
    where A: Actor,
{
    /// The actor.
    actor: A,
    /// Whether or not the actor has returned `Poll::Ready` and is ready to
    /// handle another message.
    ready_for_msg: bool,
    /// Inbox of the actor, shared between an `ActorProcess` and zero or more
    /// `ActorRef`s.
    inbox: Rc<RefCell<MailBox<A::Message>>>,
}

impl<A> ActorProcess<A>
    where A: Actor,
{
    /// Create a new actor process.
    pub fn new(pid: ProcessId, actor: A, _options: ActorOptions, system_ref: ActorSystemRef) -> ActorProcess<A> {
        ActorProcess {
            actor,
            ready_for_msg: false,
            inbox: Rc::new(RefCell::new(MailBox::new(pid, system_ref))),
        }
    }

    /// Create a new reference to this actor.
    pub fn create_ref(&self) -> ActorRef<A> {
        ActorRef::new(Rc::downgrade(&self.inbox))
    }

    /// Calls `Actor.poll`.
    ///
    /// If the return value is some that it should be returned, otherwise the
    /// loop in `run` can continue.
    ///
    /// Assumes `ready_for_msg` to be false.
    fn poll_actor(&mut self, ctx: &mut ActorContext) -> Option<ProcessResult> {
        trace!("polling actor");
        debug_assert!(!self.ready_for_msg);
        match self.actor.poll(ctx) {
            Poll::Ready(Ok(Status::Complete)) => Some(ProcessResult::Complete),
            Poll::Ready(Ok(Status::Ready)) => {
                self.ready_for_msg = true;
                None
            },
            // TODO: send error to supervisor.
            Poll::Ready(Err(_err)) => Some(ProcessResult::Complete),
            Poll::Pending => Some(ProcessResult::Pending),
        }
    }

    /// Tries to receive a message and deliver to the actor.
    ///
    /// Returned value is the same as in `poll_actor`.
    ///
    /// Assumes `ready_for_msg` to be true.
    fn handle_msg(&mut self, ctx: &mut ActorContext) -> Option<ProcessResult> {
        debug_assert!(self.ready_for_msg);
        // Retrieve another message, if any.
        let msg = match self.inbox.try_borrow_mut() {
            Ok(mut inbox) => match inbox.receive() {
                Some(msg) => msg,
                None => return Some(ProcessResult::Pending),
            },
            Err(_) => unreachable!("can't retrieve message, inbox already borrowed"),
        };

        // And deliver the message to the actor.
        trace!("delivering message to actor");
        match self.actor.handle(ctx, msg) {
            Poll::Ready(Ok(Status::Complete)) => Some(ProcessResult::Complete),
            Poll::Ready(Ok(Status::Ready)) => None,
            // TODO: send error to supervisor.
            Poll::Ready(Err(_err)) => Some(ProcessResult::Complete),
            Poll::Pending => {
                self.ready_for_msg = false;
                Some(ProcessResult::Pending)
            },
        }
    }
}

impl<A> Process for ActorProcess<A>
    where A: Actor,
{
    fn run(&mut self, _system_ref: &mut ActorSystemRef) -> ProcessResult {
        trace!("running actor process");
        // Create our actor execution context.
        let mut ctx = ActorContext{};

        loop {
            if self.ready_for_msg {
                if let Some(result) = self.handle_msg(&mut ctx) {
                    return result;
                }
            } else if let Some(result) = self.poll_actor(&mut ctx) {
                return result;
            }
        }
    }
}

impl<A> fmt::Debug for ActorProcess<A>
    where A: Actor,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ActorProcess")
            .field("ready_for_msg", &self.ready_for_msg)
            .finish()
    }
}
