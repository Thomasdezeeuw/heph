//! Tests related to `ActorRef`.

use heph::actor_ref::{ActorRef, Join, RpcError, SendError, SendValue};

use crate::util::{assert_send, assert_size, assert_sync};

#[test]
fn size() {
    assert_size::<ActorRef<()>>(24);
    assert_size::<SendValue<'_, ()>>(40);
    assert_size::<Join<'_, ()>>(32);
}

#[test]
fn is_send_sync() {
    assert_send::<ActorRef<()>>();
    assert_sync::<ActorRef<()>>();

    // UnsafeCell is !Sync and Send, our reference should still be Send and
    // Sync.
    assert_send::<ActorRef<std::cell::UnsafeCell<()>>>();
    assert_sync::<ActorRef<std::cell::UnsafeCell<()>>>();
}

#[test]
fn send_value_future_is_send_sync() {
    assert_send::<SendValue<()>>();
    assert_sync::<SendValue<()>>();
}

#[test]
fn send_error_format() {
    assert_eq!(format!("{}", SendError), "unable to send message");
}

#[test]
fn rpc_error_format() {
    assert_eq!(format!("{}", RpcError::SendError), "unable to send message");
    assert_eq!(format!("{}", RpcError::SendError), format!("{}", SendError));
    assert_eq!(format!("{}", RpcError::NoResponse), "no RPC response");
}
