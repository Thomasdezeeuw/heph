//! Tests related to `ActorGroup`.

use heph::actor_ref::{ActorGroup, Delivery};

use crate::util::{assert_send, assert_size, assert_sync};

#[test]
fn size() {
    assert_size::<ActorGroup<()>>(32);
}

#[test]
fn is_send_sync() {
    assert_send::<ActorGroup<()>>();
    assert_sync::<ActorGroup<()>>();

    // UnsafeCell is !Sync and Send, our reference should still be Send and
    // Sync.
    assert_send::<ActorGroup<std::cell::UnsafeCell<()>>>();
    assert_sync::<ActorGroup<std::cell::UnsafeCell<()>>>();
}

#[test]
fn empty() {
    let group = ActorGroup::<()>::empty();
    assert!(group.try_send((), Delivery::ToAll).is_err());
    assert!(group.try_send((), Delivery::ToOne).is_err());
    assert_eq!(group.len(), 0);
    assert!(group.is_empty());
}

#[test]
fn make_unique_empty() {
    let mut group = ActorGroup::<()>::empty();
    assert_eq!(group.len(), 0);
    group.make_unique();
    assert_eq!(group.len(), 0);
}
