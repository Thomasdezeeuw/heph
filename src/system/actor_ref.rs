//! Module containing the `ActorRef` and related types.

use std::cell::RefCell;
use std::rc::Rc;

use system::actor_process::MailBox;
use system::error::SendError;

// TODO: add examples on how to share the reference and how to send messages.

/// A reference to an actor inside a running [`ActorSystem`].
///
/// This reference can be used to send messages to the actor. To share this
/// reference simply clone it.
///
/// [`ActorSystem`]: struct.ActorSystem.html
#[derive(Debug)]
pub struct ActorRef<M> {
    inbox: Rc<RefCell<MailBox<M>>>,
}

impl<M> ActorRef<M> {
    /// Send a message to the actor.
    pub fn send<Msg>(&mut self, msg: Msg) -> Result<(), SendError<M>>
        where Msg: Into<M>,
    {
        self.inbox.borrow_mut().deliver(msg.into())
    }
}

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> ActorRef<M> {
        ActorRef {
            inbox: Rc::clone(&self.inbox),
        }
    }
}
