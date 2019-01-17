//! Tests for the actor references.

use crate::actor_ref::{ActorShutdown, LocalActorRef, SendError};
use crate::mailbox::MailBox;
use crate::scheduler::ProcessId;
use crate::test;
use crate::util::Shared;

#[test]
fn local_actor_ref() {
    // Create our mailbox.
    let pid = ProcessId(0);
    let system_ref = test::system_ref();
    let mut mailbox = Shared::new(MailBox::new(pid, system_ref));
    assert_eq!(mailbox.borrow_mut().receive(), None);

    // Create our actor reference.
    let mut actor_ref = LocalActorRef::new(mailbox.downgrade());

    // Send a message.
    actor_ref.send(()).unwrap();
    assert_eq!(mailbox.borrow_mut().receive(), Some(()));

    // Clone the reference should send to the same mailbox.
    let mut actor_ref2 = actor_ref.clone();
    actor_ref2.send(()).unwrap();
    assert_eq!(mailbox.borrow_mut().receive(), Some(()));

    // Dropping the mailbox should cause send messages to return errors.
    assert_eq!(mailbox.borrow_mut().receive(), None);
    drop(mailbox);
    assert_eq!(actor_ref.send(()), Err(SendError { message: () }));
    assert_eq!(actor_ref2.send(()), Err(SendError { message: () }));
}

#[test]
fn machine_local_actor_ref() {
    // Create our mailbox.
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut mailbox = Shared::new(MailBox::new(pid, system_ref.clone()));
    assert_eq!(mailbox.borrow_mut().receive(), None);

    // Create our actor reference.
    let mut actor_ref = LocalActorRef::new(mailbox.downgrade())
        .upgrade(&mut system_ref).unwrap();

    // Send a message.
    actor_ref.send(());
    assert_eq!(mailbox.borrow_mut().receive(), Some(()));

    // After the mailbox is dropped the local reference should return an error
    // when trying to upgrade.
    let local_actor_ref = LocalActorRef::new(mailbox.downgrade());
    drop(mailbox);
    assert_eq!(local_actor_ref.upgrade(&mut system_ref).unwrap_err(), ActorShutdown);
}
