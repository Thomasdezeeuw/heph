//! Tests for the actor references.

use std::convert::TryFrom;
use std::mem::size_of;

use crossbeam_channel as channel;

use crate::actor_ref::{ActorGroup, ActorRef};
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
    assert_eq!(size_of::<ActorRef<()>>(), 56);
    assert_eq!(size_of::<ActorGroup<()>>(), 24);
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

#[test]
fn empty_actor_group() {
    let group = ActorGroup::<()>::empty();
    assert!(group.send(()).is_err());
}

#[test]
fn new_actor_group() {
    let actor_refs: Vec<ActorRef<()>> = (0..3)
        .into_iter()
        .map(|pid| {
            let pid = ProcessId(pid);
            let (_, inbox_ref) = Inbox::new(test::new_waker(pid));
            ActorRef::from_inbox(inbox_ref)
        })
        .collect();

    let group = ActorGroup::new(actor_refs);
    assert!(group.send(()).is_ok());
}

#[test]
fn new_actor_group_from_iter() {
    let group: ActorGroup<()> = (0..3)
        .into_iter()
        .map(|pid| {
            let pid = ProcessId(pid);
            let (_, inbox_ref) = Inbox::new(test::new_waker(pid));
            ActorRef::from_inbox(inbox_ref)
        })
        .collect();
    assert!(group.send(()).is_ok());
}

#[test]
fn add_actor_to_empty_group() {
    let pid = ProcessId(0);
    let (mut inbox, inbox_ref) = Inbox::new(test::new_waker(pid));
    let actor_ref = ActorRef::from_inbox(inbox_ref);

    let mut group = ActorGroup::<usize>::empty();
    group.add(actor_ref);

    group <<= 1usize;
    assert_eq!(inbox.receive_next(), Some(1));
    assert_eq!(inbox.receive_next(), None);
}

#[test]
fn add_actor_to_group() {
    let mut inboxes = Vec::new();
    let mut actor_refs = Vec::new();
    for pid in 0..3 {
        let pid = ProcessId(pid);
        let (inbox, inbox_ref) = Inbox::new(test::new_waker(pid));
        let actor_ref = ActorRef::from_inbox(inbox_ref);
        inboxes.push(inbox);
        actor_refs.push(actor_ref);
    }
    let mut group = ActorGroup::new(actor_refs);

    let pid = ProcessId(4);
    let (inbox, inbox_ref) = Inbox::new(test::new_waker(pid));
    let actor_ref = ActorRef::from_inbox(inbox_ref);
    inboxes.push(inbox);
    group.add(actor_ref);

    group <<= 1usize;
    for inbox in inboxes.iter_mut() {
        assert_eq!(inbox.receive_next(), Some(1));
        assert_eq!(inbox.receive_next(), None);
    }
}

#[test]
fn extend_empty_actor_group() {
    let mut group = ActorGroup::empty();

    let mut inboxes = Vec::new();
    let mut actor_refs = Vec::new();
    for pid in 0..3 {
        let pid = ProcessId(pid);
        let (inbox, inbox_ref) = Inbox::new(test::new_waker(pid));
        let actor_ref = ActorRef::from_inbox(inbox_ref);
        inboxes.push(inbox);
        actor_refs.push(actor_ref);
    }

    group.extend(actor_refs);

    group <<= 1usize;
    for inbox in inboxes.iter_mut() {
        assert_eq!(inbox.receive_next(), Some(1));
        assert_eq!(inbox.receive_next(), None);
    }
}

#[test]
fn extend_actor_group() {
    let mut inboxes = Vec::new();
    let mut actor_refs = Vec::new();
    for pid in 0..3 {
        let pid = ProcessId(pid);
        let (inbox, inbox_ref) = Inbox::new(test::new_waker(pid));
        let actor_ref = ActorRef::from_inbox(inbox_ref);
        inboxes.push(inbox);
        actor_refs.push(actor_ref);
    }

    let mut group = ActorGroup::new(actor_refs);

    let mut actor_refs = Vec::new();
    for pid in 0..3 {
        let pid = ProcessId(pid);
        let (inbox, inbox_ref) = Inbox::new(test::new_waker(pid));
        let actor_ref = ActorRef::from_inbox(inbox_ref);
        inboxes.push(inbox);
        actor_refs.push(actor_ref);
    }

    group.extend(actor_refs);

    group <<= 1usize;
    for inbox in inboxes.iter_mut() {
        assert_eq!(inbox.receive_next(), Some(1));
        assert_eq!(inbox.receive_next(), None);
    }
}

#[test]
fn send_to_empty_actor_group() {
    let group = ActorGroup::<usize>::empty();
    assert!(group.send(1usize).is_err());
}

#[test]
fn send_to_actor_group() {
    let mut group = ActorGroup::<u32>::empty();

    // Kind: `ActorRefKind::Node`.
    let pid = ProcessId(0);
    let (mut inbox1, inbox_ref) = Inbox::new(test::new_waker(pid));
    group.add(ActorRef::from_inbox(inbox_ref));

    // Kind: `ActorRefKind::Sync`.
    let (send, recv1) = channel::unbounded();
    group.add(ActorRef::for_sync_actor(send));

    // Kind: `ActorRefKind::Node` -> `ActorRefKind::Mapped`.
    let pid = ProcessId(1);
    let (mut inbox2, inbox_ref) = Inbox::<u64>::new(test::new_waker(pid));
    group.add(ActorRef::from_inbox(inbox_ref).map());

    // Kind: `ActorRefKind::Node` -> `ActorRefKind::TryMapped`.
    let pid = ProcessId(2);
    let (mut inbox3, inbox_ref) = Inbox::<u16>::new(test::new_waker(pid));
    group.add(ActorRef::from_inbox(inbox_ref).try_map());

    // Kind: `ActorRefKind::Sync` -> `ActorRefKind::Mapped`.
    let (send, recv2) = channel::unbounded::<u64>();
    group.add(ActorRef::for_sync_actor(send).map());

    // Kind: `ActorRefKind::Sync` -> `ActorRefKind::TryMapped`.
    let (send, recv3) = channel::unbounded::<u16>();
    group.add(ActorRef::for_sync_actor(send).try_map());

    // Send the message.
    group <<= 1u32;

    // Kind: `ActorRefKind::Node`.
    assert_eq!(inbox1.receive_next(), Some(1));
    assert_eq!(inbox1.receive_next(), None);

    // Kind: `ActorRefKind::Sync`.
    assert_eq!(recv1.try_recv(), Ok(1));
    assert_eq!(recv1.try_recv(), Err(channel::TryRecvError::Empty));

    // Kind: `ActorRefKind::Node` -> `ActorRefKind::Mapped`.
    assert_eq!(inbox2.receive_next(), Some(1));
    assert_eq!(inbox2.receive_next(), None);

    // Kind: `ActorRefKind::Node` -> `ActorRefKind::TryMapped`.
    assert_eq!(inbox3.receive_next(), Some(1));
    assert_eq!(inbox3.receive_next(), None);

    // Kind: `ActorRefKind::Sync` -> `ActorRefKind::Mapped`.
    assert_eq!(recv2.try_recv(), Ok(1));
    assert_eq!(recv2.try_recv(), Err(channel::TryRecvError::Empty));

    // Kind: `ActorRefKind::Sync` -> `ActorRefKind::TryMapped`.
    assert_eq!(recv3.try_recv(), Ok(1));
    assert_eq!(recv3.try_recv(), Err(channel::TryRecvError::Empty));
}
