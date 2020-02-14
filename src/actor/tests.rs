//! Tests for the actor module.

use std::pin::Pin;
use std::task::Poll;

use crate::actor;
use crate::inbox::Inbox;
use crate::rt::ProcessId;
use crate::test;

#[test]
fn test_actor_context() {
    let pid = ProcessId(0);
    let runtime_ref = test::runtime();
    let waker = runtime_ref.new_waker(pid);
    let (inbox, inbox_ref) = Inbox::new(waker);
    let mut ctx = actor::Context::new(pid, runtime_ref, inbox, inbox_ref);

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
