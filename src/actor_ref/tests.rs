//! Tests for the actor references.

use std::mem::size_of;

use crate::actor_ref::{ActorRef, ActorShutdown, Local, LocalMap, Machine, Map, SendError};
use crate::inbox::Inbox;
use crate::system::ProcessId;
use crate::test;

// FIXME: add a test that local actor reference is !Send and !Sync. Below
// doesn't work. :(
// fn assert_not_send<T: !Send + !Sync>(_: &T) {}

fn assert_send<T: Send + Sync>() {}

#[test]
fn size_assertions() {
    assert_eq!(
        size_of::<ActorRef<Local<usize>>>(),
        size_of::<Local<usize>>()
    );
    assert_eq!(
        size_of::<ActorRef<Machine<usize>>>(),
        size_of::<Machine<usize>>()
    );
    use crate::actor_ref::Sync;
    assert_eq!(size_of::<ActorRef<Sync<usize>>>(), size_of::<Sync<usize>>());
}

#[test]
fn local_actor_ref() {
    // Create our inbox.
    let pid = ProcessId(0);
    let system_ref = test::system_ref();
    let mut inbox = Inbox::new(pid, system_ref.clone());
    assert_eq!(inbox.receive_next(), None);

    // Create our actor reference.
    let mut actor_ref = ActorRef::new_local(inbox.create_ref());

    // Send a message.
    actor_ref.send(1).unwrap();
    assert_eq!(inbox.receive_next(), Some(1));

    // Sending multiple messages.
    actor_ref.send(2).unwrap();
    actor_ref.send(3).unwrap();
    assert_eq!(inbox.receive_next(), Some(2));
    assert_eq!(inbox.receive_next(), Some(3));

    // Clone the reference should send to the same inbox.
    let mut actor_ref2 = actor_ref.clone();
    actor_ref.send(4).unwrap();
    actor_ref2 <<= 5;
    assert_eq!(inbox.receive_next(), Some(4));
    assert_eq!(inbox.receive_next(), Some(5));

    // Should be able to compare references.
    assert_eq!(actor_ref, actor_ref2);
    let inbox2 = Inbox::new(pid, system_ref);
    let actor_ref3 = ActorRef::new_local(inbox2.create_ref());
    assert_ne!(actor_ref, actor_ref3);
    assert_ne!(actor_ref2, actor_ref3);
    assert_eq!(actor_ref3, actor_ref3);

    // Test Debug implementation.
    assert_eq!(format!("{:?}", actor_ref), "LocalActorRef");

    // Dropping the inbox should cause send messages to return errors.
    assert_eq!(inbox.receive_next(), None);
    drop(inbox);
    assert_eq!(actor_ref.send(10), Err(SendError { message: 10 }));
    assert_eq!(actor_ref2.send(11), Err(SendError { message: 11 }));
}

#[test]
fn machine_local_actor_ref() {
    // Create our inbox.
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut inbox = Inbox::new(pid, system_ref.clone());
    assert_eq!(inbox.receive_next(), None);

    // Create our actor reference.
    let mut actor_ref = ActorRef::new_local(inbox.create_ref())
        .upgrade(&mut system_ref)
        .unwrap();

    // Sending a message.
    actor_ref.send(1).unwrap();
    actor_ref <<= 2;
    assert_eq!(inbox.receive_next(), Some(1));
    assert_eq!(inbox.receive_next(), Some(2));

    // Cloning should send to the same inbox.
    let mut actor_ref2 = actor_ref.clone();
    actor_ref2 <<= 3;
    assert_eq!(inbox.receive_next(), Some(3));

    // Comparing should be possible.
    assert_eq!(actor_ref, actor_ref2);

    let inbox2 = Inbox::new(pid, system_ref.clone());
    let actor_ref3 = ActorRef::new_local(inbox2.create_ref())
        .upgrade(&mut system_ref)
        .unwrap();
    assert_ne!(actor_ref, actor_ref3);
    assert_ne!(actor_ref2, actor_ref3);

    // Test Debug implementation.
    assert_eq!(format!("{:?}", actor_ref), "MachineLocalActorRef");

    // Test Send and Sync.
    assert_send::<ActorRef<Machine<()>>>();
    // UnsafeCell is !Sync and Send, our reference should still be Send and
    // Sync.
    assert_send::<ActorRef<Machine<std::cell::UnsafeCell<()>>>>();

    // After the inbox is dropped the local reference should return an error
    // when trying to upgrade.
    let local_actor_ref = ActorRef::new_local(inbox.create_ref());
    drop(inbox);
    let res = local_actor_ref.upgrade(&mut system_ref).unwrap_err();
    assert_eq!(res, ActorShutdown);
}

#[test]
fn local_and_machine_actor_ref() {
    // Create our inbox.
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut inbox = Inbox::new(pid, system_ref.clone());
    assert_eq!(inbox.receive_next(), None);

    // Create our actor reference.
    let mut local_actor_ref = ActorRef::new_local(inbox.create_ref());
    let mut machine_actor_ref = local_actor_ref.clone().upgrade(&mut system_ref).unwrap();

    // Send a number messages via both the local and machine references.
    local_actor_ref <<= 1;
    machine_actor_ref <<= 2;
    machine_actor_ref <<= 3;
    local_actor_ref <<= 4;
    // On the first call to receive all the non-local messages are appended to
    // the local messages. Which means that local message are queue before the
    // non-local messages.
    assert_eq!(inbox.receive_next(), Some(1));
    assert_eq!(inbox.receive_next(), Some(4));
    assert_eq!(inbox.receive_next(), Some(2));
    assert_eq!(inbox.receive_next(), Some(3));
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

impl Into<Msg> for M {
    fn into(self) -> Msg {
        Msg(self.0)
    }
}

#[test]
fn local_mapped_actor_ref_different_types() {
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut inbox = Inbox::<M>::new(pid, system_ref.clone());

    let local_actor_ref = ActorRef::new_local(inbox.create_ref());
    let machine_actor_ref = local_actor_ref
        .clone()
        .upgrade(&mut system_ref)
        .expect("unable to upgrade actor reference");

    // TODO: add Sync actor reference.

    let actor_refs: &mut [ActorRef<LocalMap<Msg>>] =
        &mut [local_actor_ref.local_map(), machine_actor_ref.local_map()];

    for actor_ref in actor_refs.iter_mut() {
        actor_ref.send(Msg(1)).expect("unable to send message");
    }

    assert_eq!(inbox.receive_next(), Some(M(1))); // Local ref.
    assert_eq!(inbox.receive_next(), Some(M(1))); // Machine ref.
}

#[test]
fn local_mapped_actor_ref_different_references() {
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();

    let mut inbox1 = Inbox::<M>::new(pid, system_ref.clone());
    let local_actor_ref1 = ActorRef::new_local(inbox1.create_ref());
    let machine_actor_ref1 = local_actor_ref1
        .clone()
        .upgrade(&mut system_ref)
        .expect("unable to upgrade actor reference");

    let mut inbox2 = Inbox::<M>::new(pid, system_ref.clone());
    let local_actor_ref2 = ActorRef::new_local(inbox2.create_ref());
    let machine_actor_ref2 = local_actor_ref2
        .clone()
        .upgrade(&mut system_ref)
        .expect("unable to upgrade actor reference");

    let actor_refs: &mut [ActorRef<LocalMap<Msg>>] = &mut [
        local_actor_ref1.local_map(),
        machine_actor_ref1.local_map(),
        local_actor_ref2.local_map(),
        machine_actor_ref2.local_map(),
    ];

    for actor_ref in actor_refs.iter_mut() {
        actor_ref.send(Msg(1)).expect("unable to send message");
    }

    assert_eq!(inbox1.receive_next(), Some(M(1))); // Local ref.
    assert_eq!(inbox1.receive_next(), Some(M(1))); // Machine ref.
    assert_eq!(inbox2.receive_next(), Some(M(1))); // Local ref.
    assert_eq!(inbox2.receive_next(), Some(M(1))); // Machine ref.
}

#[test]
fn mapped_actor_ref_is_send_sync() {
    // Test Send and Sync.
    assert_send::<ActorRef<Map<()>>>();
    // UnsafeCell is !Sync and Send, our reference should still be Send and
    // Sync.
    assert_send::<ActorRef<Map<std::cell::UnsafeCell<()>>>>();
}

#[test]
fn mapped_actor_ref_different_types() {
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();
    let mut inbox = Inbox::<M>::new(pid, system_ref.clone());

    let local_actor_ref = ActorRef::new_local(inbox.create_ref());
    let machine_actor_ref = local_actor_ref
        .upgrade(&mut system_ref)
        .expect("unable to upgrade actor reference");

    // TODO: add Sync actor reference.

    let actor_refs: &mut [ActorRef<LocalMap<Msg>>] = &mut [machine_actor_ref.local_map()];

    for actor_ref in actor_refs.iter_mut() {
        actor_ref.send(Msg(1)).expect("unable to send message");
    }

    assert_eq!(inbox.receive_next(), Some(M(1))); // local ref.
}

#[test]
fn mapped_actor_ref_different_references() {
    let pid = ProcessId(0);
    let mut system_ref = test::system_ref();

    let mut inbox1 = Inbox::<M>::new(pid, system_ref.clone());
    let local_actor_ref1 = ActorRef::new_local(inbox1.create_ref());
    let machine_actor_ref1 = local_actor_ref1
        .upgrade(&mut system_ref)
        .expect("unable to upgrade actor reference");

    let mut inbox2 = Inbox::<M>::new(pid, system_ref.clone());
    let local_actor_ref2 = ActorRef::new_local(inbox2.create_ref());
    let machine_actor_ref2 = local_actor_ref2
        .upgrade(&mut system_ref)
        .expect("unable to upgrade actor reference");

    let actor_refs: &mut [ActorRef<LocalMap<Msg>>] = &mut [
        machine_actor_ref1.local_map(),
        machine_actor_ref2.local_map(),
    ];

    for actor_ref in actor_refs.iter_mut() {
        actor_ref.send(Msg(1)).expect("unable to send message");
    }

    assert_eq!(inbox1.receive_next(), Some(M(1))); // Machine ref.
    assert_eq!(inbox2.receive_next(), Some(M(1))); // Machine ref.
}
