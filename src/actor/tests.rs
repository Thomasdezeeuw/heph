//! Tests for the actor module.

use std::pin::Pin;
use std::task::Poll;

use crate::actor::ActorContext;
use crate::mailbox::MailBox;
use crate::scheduler::ProcessId;
use crate::test;
use crate::util::Shared;

#[test]
fn test_actor_context() {
    let pid = ProcessId(0);
    let system_ref = test::system_ref();
    let inbox = Shared::new(MailBox::new(pid, system_ref.clone()));
    let mut ctx = ActorContext::new(pid, system_ref, inbox);

    assert_eq!(ctx.pid(), pid);
    let mut actor_ref = ctx.actor_ref();

    // Initially the mailbox should be empty.
    let mut recv_future = ctx.receive();
    assert_eq!(test::poll_future(Pin::new(&mut recv_future)), Poll::Pending);

    // Send my self a message, and we should be able to retrieve it.
    actor_ref <<= ();
    assert_eq!(test::poll_future(Pin::new(&mut recv_future)), Poll::Ready(()));
}
