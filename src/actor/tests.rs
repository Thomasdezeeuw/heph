//! Tests for the actor module.

use std::pin::Pin;
use std::task::Poll;

use crate::actor::{self, context};
use crate::inbox::Inbox;
use crate::rt::ProcessId;
use crate::test;

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[test]
fn thread_safe_actor_context_is_send() {
    assert_send::<actor::Context<(), context::ThreadSafe>>();
}

#[test]
fn thread_safe_actor_context_is_sync() {
    assert_sync::<actor::Context<(), context::ThreadSafe>>();
}

#[test]
fn test_local_actor_context() {
    let pid = ProcessId(0);
    let runtime_ref = test::runtime();
    let waker = runtime_ref.new_waker(pid);
    let (inbox, inbox_ref) = Inbox::new(waker);
    let mut ctx = actor::Context::new_local(pid, inbox, inbox_ref, runtime_ref);

    assert_eq!(ctx.pid(), pid);
    let mut actor_ref = ctx.actor_ref();

    // Initially the mailbox should be empty.
    let mut recv_future = ctx.receive_next();
    assert_eq!(test::poll_future(Pin::new(&mut recv_future)), Poll::Pending);

    // Send my self a message, and we should be able to retrieve it.
    actor_ref <<= ();
    let res = test::poll_future(Pin::new(&mut recv_future));
    assert_eq!(res, Poll::Ready(()));
}
