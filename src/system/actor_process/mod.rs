//! Module containing the implementation of the `Process` trait for `Actor`s.

mod mailbox;
mod actor_ref;

pub use self::mailbox::MailBox;
pub use self::actor_ref::ActorRef;
