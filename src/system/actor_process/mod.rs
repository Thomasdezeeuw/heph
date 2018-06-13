//! Module containing the implementation of the `Process` trait for `Actor`s.

mod mailbox;
mod actor_ref;

pub use self::mailbox::MailBox;
pub use self::actor_ref::ActorRef;

/// The mailbox of an `Actor` that is shared between an `ActorProcess` and one
/// or more `ActorRef`s.
type SharedMailbox<M> = Rc<RefCell<MailBox<M>>>;
