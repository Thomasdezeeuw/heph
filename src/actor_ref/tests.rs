//! Tests for the actor references.

use std::mem::size_of;

use crate::actor_ref::{ActorShutdown, ActorRef, ActorRefType, LocalActorRef, Local, Machine, SendError};
use crate::mailbox::MailBox;
use crate::scheduler::ProcessId;
use crate::test;
use crate::util::Shared;

// FIXME: add a test that local actor reference is !Send and !Sync. Below
// doesn't work. :(
//fn assert_not_send<T: !Send + !Sync>(_: &T) {}

#[test]
fn size_assertions() {
    assert_eq!(size_of::<ActorRef<usize, Local>>(), size_of::<<Local as ActorRefType<usize>>::Data>());
    assert_eq!(size_of::<ActorRef<usize, Machine>>(), size_of::<<Machine as ActorRefType<usize>>::Data>());
}

#[test]
fn local_actor_ref() {
    // Create our mailbox.
    let pid = ProcessId(0);
    let system_ref = test::system_ref();
    let mut mailbox = Shared::new(MailBox::new(pid, system_ref.clone()));
    assert_eq!(mailbox.borrow_mut().receive(), None);

    // Create our actor reference.
    let mut actor_ref = LocalActorRef::new(mailbox.downgrade());

    // Send a message.
    actor_ref.send(1).unwrap();
    assert_eq!(mailbox.borrow_mut().receive(), Some(1));

    // Sending multiple messages.
    actor_ref.send(2).unwrap();
    actor_ref.send(3).unwrap();
    assert_eq!(mailbox.borrow_mut().receive(), Some(2));
    assert_eq!(mailbox.borrow_mut().receive(), Some(3));

    // Clone the reference should send to the same mailbox.
    let mut actor_ref2 = actor_ref.clone();
    actor_ref.send(4).unwrap();
    actor_ref2 <<= 5;
    assert_eq!(mailbox.borrow_mut().receive(), Some(4));
    assert_eq!(mailbox.borrow_mut().receive(), Some(5));

    // Should be able to compare references.
    assert_eq!(actor_ref, actor_ref2);
    let mailbox2 = Shared::new(MailBox::new(pid, system_ref));
    let actor_ref3 = LocalActorRef::new(mailbox2.downgrade());
    assert_ne!(actor_ref, actor_ref3);
    assert_ne!(actor_ref2, actor_ref3);
    assert_eq!(actor_ref3, actor_ref3);

    // Test Debug implementation.
    assert_eq!(format!("{:?}", actor_ref), "LocalActorRef");

    // Dropping the mailbox should cause send messages to return errors.
    assert_eq!(mailbox.borrow_mut().receive(), None);
    drop(mailbox);
    assert_eq!(actor_ref.send(10), Err(SendError { message: 10 }));
    assert_eq!(actor_ref2.send(11), Err(SendError { message: 11 }));
}

fn assert_send<T: Send + Sync>(_: &T) {}

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

    // Sending a message.
    actor_ref.send(1).unwrap();
    actor_ref <<= 2;
    assert_eq!(mailbox.borrow_mut().receive(), Some(1));
    assert_eq!(mailbox.borrow_mut().receive(), Some(2));

    // Cloning should send to the same mailbox.
    let mut actor_ref2 = actor_ref.clone();
    actor_ref2 <<= 3;
    assert_eq!(mailbox.borrow_mut().receive(), Some(3));

    /* TODO: depends on https://github.com/crossbeam-rs/crossbeam/issues/319.
    // Comparing should be possible.
    assert_eq!(actor_ref, actor_ref2);

    let mailbox2 = Shared::new(MailBox::new(pid, system_ref.clone()));
    let actor_ref3 = LocalActorRef::new(mailbox2.downgrade())
        .upgrade(&mut system_ref).unwrap();
    assert_ne!(actor_ref, actor_ref3);
    assert_ne!(actor_ref2, actor_ref3);
    */

    // Test Debug implementation.
    assert_eq!(format!("{:?}", actor_ref), "MachineLocalActorRef");

    // Test Send and Sync.
    assert_send(&actor_ref);

    // After the mailbox is dropped the local reference should return an error
    // when trying to upgrade.
    let local_actor_ref = LocalActorRef::new(mailbox.downgrade());
    drop(mailbox);
    assert_eq!(local_actor_ref.upgrade(&mut system_ref).unwrap_err(), ActorShutdown);
}

#[test]
fn local_and_machine_actor_ref() {
    // Create our mailbox.
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut mailbox = Shared::new(MailBox::new(pid, system_ref.clone()));
    assert_eq!(mailbox.borrow_mut().receive(), None);

    // Create our actor reference.
    let mut local_actor_ref = LocalActorRef::new(mailbox.downgrade());
    let mut machine_actor_ref = local_actor_ref.clone().upgrade(&mut system_ref).unwrap();

    // Send a number messages via both the local and machine references.
    local_actor_ref <<= 1;
    machine_actor_ref <<= 2;
    machine_actor_ref <<= 3;
    local_actor_ref <<= 4;
    // On the first call to receive all the non-local messages are appended to
    // the local messages. Which means that local message are queue before the
    // non-local messages.
    assert_eq!(mailbox.borrow_mut().receive(), Some(1));
    assert_eq!(mailbox.borrow_mut().receive(), Some(4));
    assert_eq!(mailbox.borrow_mut().receive(), Some(2));
    assert_eq!(mailbox.borrow_mut().receive(), Some(3));
}
