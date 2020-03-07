//! Tests for the actor references.

use std::convert::TryFrom;
use std::mem::size_of;

use crossbeam_channel as channel;

use crate::actor_ref::ActorRef;
use crate::inbox::Inbox;
use crate::rt::ProcessId;
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
    assert_eq!(size_of::<ActorRef<()>>(), 40);
}

#[test]
fn actor_ref() {
    let pid = ProcessId(0);
    let waker = test::new_waker(pid);
    let (mut inbox, inbox_ref) = Inbox::new(waker);
    let mut actor_ref = ActorRef::from_inbox(inbox_ref.clone());

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
    let waker = test::new_waker(pid);
    let (mut inbox, inbox_ref) = Inbox::new(waker);

    let mut actor_ref = ActorRef::from_inbox(inbox_ref);

    // Send a number messages.
    actor_ref <<= 1;
    actor_ref <<= 2;
    actor_ref <<= 3;
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
fn mapped_actor_ref() {
    let pid = ProcessId(0);
    let waker = test::new_waker(pid);
    let (mut inbox, inbox_ref) = Inbox::<M>::new(waker);

    let actor_ref = ActorRef::from_inbox(inbox_ref);
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
    let waker = test::new_waker(pid);
    let (mut inbox, inbox_ref) = Inbox::<M>::new(waker);

    let actor_ref = ActorRef::from_inbox(inbox_ref);
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
