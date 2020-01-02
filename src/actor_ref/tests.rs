//! Tests for the actor references.

use std::convert::TryFrom;
use std::mem::size_of;

use crossbeam_channel as channel;

use crate::actor_ref::{ActorRef, LocalActorRef};
use crate::inbox::Inbox;
use crate::system::ProcessId;
use crate::test;

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[test]
fn mapped_actor_ref_is_send_sync() {
    assert_send::<ActorRef<()>>();
    assert_sync::<ActorRef<()>>();

    // UnsafeCell is !Sync and Send, our reference should still be Send and
    // Sync.
    assert_send::<ActorRef<std::cell::UnsafeCell<()>>>();
    assert_sync::<ActorRef<std::cell::UnsafeCell<()>>>();
}

#[test]
fn size_assertions() {
    assert_eq!(size_of::<LocalActorRef<()>>(), 24);
    // ActorRef is quite big, maybe replace the `task::Waker` in `Node` will
    // reduce the size.
    assert_eq!(size_of::<ActorRef<()>>(), 40);
}

#[test]
fn local_actor_ref() {
    let pid = ProcessId(0);
    let system_ref = test::system_ref();
    let mut inbox = Inbox::new(pid, system_ref);
    let mut actor_ref = LocalActorRef::from_inbox(inbox.create_ref());

    // Send a message.
    actor_ref.send(1).unwrap();
    assert_eq!(inbox.receive_next(), Some(1));

    // Sending multiple messages.
    actor_ref.send(2).unwrap();
    actor_ref.send(3).unwrap();
    assert_eq!(inbox.receive_next(), Some(2));
    assert_eq!(inbox.receive_next(), Some(3));

    // Cloning the reference should send to the same inbox.
    let mut actor_ref2 = actor_ref.clone();
    actor_ref.send(4).unwrap();
    actor_ref2 <<= 5;
    assert_eq!(inbox.receive_next(), Some(4));
    assert_eq!(inbox.receive_next(), Some(5));

    // Test Debug implementation.
    assert_eq!(format!("{:?}", actor_ref), "LocalActorRef");

    // Dropping the inbox should cause send messages to return errors.
    assert_eq!(inbox.receive_next(), None);
    drop(inbox);
    assert!(actor_ref.send(10).is_err());
    assert!(actor_ref2.send(11).is_err());
}

#[test]
fn actor_ref() {
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut inbox = Inbox::new(pid, system_ref.clone());
    let mut actor_ref = LocalActorRef::from_inbox(inbox.create_ref())
        .upgrade(&mut system_ref)
        .unwrap();

    // Sending messages.
    actor_ref.send(1).unwrap();
    actor_ref <<= 2;
    assert_eq!(inbox.receive_next(), Some(1));
    assert_eq!(inbox.receive_next(), Some(2));

    // Cloning should send to the same inbox.
    let mut actor_ref2 = actor_ref.clone();
    actor_ref2 <<= 3;
    assert_eq!(inbox.receive_next(), Some(3));

    // Test Debug implementation.
    assert_eq!(format!("{:?}", actor_ref), "ActorRef");

    // After the inbox is dropped the local reference should return an error
    // when trying to upgrade.
    let local_actor_ref = LocalActorRef::from_inbox(inbox.create_ref());
    drop(inbox);
    assert!(local_actor_ref.upgrade(&mut system_ref).is_err());
    // Sending messages should also return an error.
    assert!(actor_ref.send(10).is_err());
}

#[test]
fn sync_actor_ref() {
    let (sender, inbox) = channel::unbounded();
    let mut actor_ref = ActorRef::for_sync_actor(sender);

    // Sending messages.
    actor_ref.send(1).unwrap();
    actor_ref <<= 2;
    assert_eq!(inbox.try_recv(), Ok(1));
    assert_eq!(inbox.try_recv(), Ok(2));

    // Cloning should send to the same inbox.
    let mut actor_ref2 = actor_ref.clone();
    actor_ref2 <<= 3;
    assert_eq!(inbox.try_recv(), Ok(3));

    // Test Debug implementation.
    assert_eq!(format!("{:?}", actor_ref), "ActorRef");

    // After the inbox is dropped sending messages should also return an error.
    drop(inbox);
    assert!(actor_ref.send(10).is_err());
}

#[test]
fn actor_ref_message_order() {
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut inbox = Inbox::new(pid, system_ref.clone());

    let mut local_actor_ref = LocalActorRef::from_inbox(inbox.create_ref());
    let mut actor_ref = local_actor_ref.clone().upgrade(&mut system_ref).unwrap();

    // Send a number messages via both the local and machine references.
    local_actor_ref <<= 1;
    actor_ref <<= 2;
    local_actor_ref <<= 3;
    actor_ref <<= 4;
    // The message should arrive in order, but this is just an best effort.
    assert_eq!(inbox.receive_next(), Some(1));
    assert_eq!(inbox.receive_next(), Some(2));
    assert_eq!(inbox.receive_next(), Some(3));
    assert_eq!(inbox.receive_next(), Some(4));
}

// Some type we can control.
#[derive(PartialEq, Debug)]
struct M(usize);

// A type we don't control, but we want to receive.
struct Msg(usize);

impl From<Msg> for M {
    fn from(msg: Msg) -> M {
        M(msg.0)
    }
}

// Another type we don't control, but we want to receive.
struct Msg2(usize);

impl TryFrom<Msg2> for M {
    type Error = ();
    fn try_from(msg: Msg2) -> Result<M, Self::Error> {
        Ok(M(msg.0 + 1))
    }
}

#[test]
fn local_mapped_actor_ref() {
    let pid = ProcessId(0);
    let system_ref = test::system_ref();
    let mut inbox = Inbox::<M>::new(pid, system_ref);

    let mut actor_ref = LocalActorRef::from_inbox(inbox.create_ref());
    let mut mapped_actor_ref: LocalActorRef<Msg> = actor_ref.clone().map();

    actor_ref.send(M(1)).expect("unable to send message");
    mapped_actor_ref
        .send(Msg(2))
        .expect("unable to send message");

    assert_eq!(inbox.receive_next(), Some(M(1))); // Local ref.
    assert_eq!(inbox.receive_next(), Some(M(2))); // Mapped ref.
    assert_eq!(inbox.receive_next(), None);
}

#[test]
fn local_try_mapped_actor_ref() {
    let pid = ProcessId(0);
    let system_ref = test::system_ref();
    let mut inbox = Inbox::<M>::new(pid, system_ref);

    let mut actor_ref = LocalActorRef::from_inbox(inbox.create_ref());
    let mut mapped_actor_ref: LocalActorRef<Msg2> = actor_ref.clone().try_map();

    actor_ref.send(M(1)).expect("unable to send message");
    mapped_actor_ref
        .send(Msg2(2))
        .expect("unable to send message");

    assert_eq!(inbox.receive_next(), Some(M(1))); // Local ref.
    assert_eq!(inbox.receive_next(), Some(M(2 + 1))); // Try mapped ref.
    assert_eq!(inbox.receive_next(), None);
}

#[test]
fn mapped_actor_ref() {
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut inbox = Inbox::<M>::new(pid, system_ref.clone());

    let local_actor_ref = LocalActorRef::from_inbox(inbox.create_ref());
    let actor_ref = local_actor_ref.upgrade(&mut system_ref).unwrap();
    let mapped_actor_ref: ActorRef<Msg> = actor_ref.clone().map();

    actor_ref.send(M(1)).expect("unable to send message");
    mapped_actor_ref
        .send(Msg(2))
        .expect("unable to send message");

    assert_eq!(inbox.receive_next(), Some(M(1))); // Actor ref.
    assert_eq!(inbox.receive_next(), Some(M(2))); // Mapped ref.
    assert_eq!(inbox.receive_next(), None);
}

#[test]
fn try_mapped_actor_ref() {
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut inbox = Inbox::<M>::new(pid, system_ref.clone());

    let local_actor_ref = LocalActorRef::from_inbox(inbox.create_ref());
    let actor_ref = local_actor_ref.upgrade(&mut system_ref).unwrap();
    let mapped_actor_ref: ActorRef<Msg2> = actor_ref.clone().try_map();

    actor_ref.send(M(1)).expect("unable to send message");
    mapped_actor_ref
        .send(Msg2(2))
        .expect("unable to send message");

    assert_eq!(inbox.receive_next(), Some(M(1))); // Actor ref.
    assert_eq!(inbox.receive_next(), Some(M(2 + 1))); // Try mapped ref.
    assert_eq!(inbox.receive_next(), None);
}

#[test]
fn mapped_sync_actor_ref() {
    let (sender, inbox) = channel::unbounded();
    let actor_ref = ActorRef::<M>::for_sync_actor(sender);
    let mapped_actor_ref: ActorRef<Msg> = actor_ref.clone().map();

    actor_ref.send(M(1)).expect("unable to send message");
    mapped_actor_ref
        .send(Msg(2))
        .expect("unable to send message");

    assert_eq!(inbox.try_recv(), Ok(M(1))); // Sync actor ref.
    assert_eq!(inbox.try_recv(), Ok(M(2))); // Mapped ref.
    assert!(inbox.try_recv().is_err());
}

#[test]
fn try_mapped_sync_actor_ref() {
    let (sender, inbox) = channel::unbounded();
    let actor_ref = ActorRef::<M>::for_sync_actor(sender);
    let mapped_actor_ref: ActorRef<Msg2> = actor_ref.clone().try_map();

    actor_ref.send(M(1)).expect("unable to send message");
    mapped_actor_ref
        .send(Msg2(2))
        .expect("unable to send message");

    assert_eq!(inbox.try_recv(), Ok(M(1))); // Actor ref.
    assert_eq!(inbox.try_recv(), Ok(M(2 + 1))); // Try mapped ref.
    assert!(inbox.try_recv().is_err());
}
