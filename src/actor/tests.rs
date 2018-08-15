//! Tests for the actor module.

use std::mem::PinMut;
use std::task::Poll;

use crate::actor::ActorContext;
use crate::mailbox::MailBox;
use crate::process::ProcessId;
use crate::test;
use crate::util::Shared;

#[test]
fn test_actor_context() {
    let pid = ProcessId(0);
    let system_ref = test::system_ref();
    let inbox = Shared::new(MailBox::new(pid, system_ref.clone()));
    let mut ctx = ActorContext::new(pid, system_ref, inbox);

    assert_eq!(ctx.pid(), pid);
    let mut self_ref = ctx.myself();

    // Initially the mailbox should be empty.
    let mut recv_future = ctx.receive();
    let mut recv_future = PinMut::new(&mut recv_future);
    assert_eq!(test::poll_future(&mut recv_future), Poll::Pending);

    // Send my self a message, and we should be able to retrieve it.
    self_ref.send(()).unwrap();
    assert_eq!(test::poll_future(&mut recv_future), Poll::Ready(()));
}
