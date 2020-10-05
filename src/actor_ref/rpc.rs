//! Types related the `ActorRef` Remote Procedure Call (RPC) mechanism.
//!
//! RPC is implemented by sending a [`RpcMessage`] to the actor, which contains
//! the request message and a [`RpcResponse`]. The `RpcResponse` allows the
//! receiving actor to send back a response to the sending actor.
//!
//! To support RPC the receiving actor needs to implement
//! [`From`]`<`[`RpcMessage`]`<Req, Res>>`, where `Req` is the type of the
//! request message and `Res` the type of the response. The RPC message can then
//! be received like any other message.
//!
//! The sending actor needs to call [`ActorRef::rpc`] with the correct request
//! type. That will return an [`Rpc`] [`Future`] which returns the response to
//! the call, or [`NoResponse`] in case the actor didn't send a response.
//!
//! [`ActorRef::rpc`]: crate::actor_ref::ActorRef::rpc
//!
//! # Examples
//!
//! Using RPC to communicate with another actor.
//!
//! ```
//! # #![feature(never_type)]
//! #
//! use heph::{actor, rt, Runtime, ActorOptions};
//! use heph::actor_ref::{ActorRef, RpcMessage, NoResponse};
//! use heph::supervisor::NoSupervisor;
//!
//! /// Message type for [`counter`].
//! struct Add(RpcMessage<usize, usize>);
//!
//! /// Required to support RPC.
//! impl From<RpcMessage<usize, usize>> for Add {
//!     fn from(msg: RpcMessage<usize, usize>) -> Add {
//!         Add(msg)
//!     }
//! }
//!
//! /// Receiving actor of the RPC.
//! async fn counter(mut ctx: actor::Context<Add>) -> Result<(), !> {
//!     // State of the counter.
//!     let mut count: usize = 0;
//!     // Receive a message like normal.
//!     let RpcMessage { request, response } = ctx.receive_next().await.0;
//!     count += request;
//!     // Send back the current state, ignoring any errors.
//!     let _ = response.respond(count);
//!     // And we're done.
//!     Ok(())
//! }
//!
//! /// Sending actor of the RPC.
//! async fn requester(mut ctx: actor::Context<!>, actor_ref: ActorRef<Add>) -> Result<(), !> {
//!     // Make the procedure call.
//!     let response = actor_ref.rpc(&mut ctx, 10).unwrap().await;
//! #   assert!(response.is_ok());
//!     match response {
//!         // We got a response.
//!         Ok(count) => println!("Current count: {}", count),
//!         // Actor failed to respond.
//!         Err(NoResponse) => eprintln!("Counter didn't reply"),
//!     }
//!     Ok(())
//! }
//!
//! # fn main() -> Result<(), rt::Error> {
//! #    Runtime::new()?.with_setup(|mut runtime_ref| {
//! #        let counter = counter as fn(_) -> _;
//! #        let actor_ref = runtime_ref.spawn_local(NoSupervisor, counter, (), ActorOptions::default());
//! #
//! #        let requester = requester as fn(_, _) -> _;
//! #        runtime_ref.spawn_local(NoSupervisor, requester, actor_ref, ActorOptions::default().mark_ready());
//! #        Ok(())
//! #    }).start()
//! # }
//! ```
//!
//! Supporting multiple procedure within the same actor is possible by making
//! the message an `enum` as the example below shows. Furthermore synchronous
//! actors are supported.
//!
//! ```
//! # #![feature(never_type)]
//! #
//! use heph::actor::sync::SyncContext;
//! use heph::actor_ref::{ActorRef, RpcMessage};
//! use heph::supervisor::NoSupervisor;
//! use heph::actor;
//! use heph::rt::{self, Runtime, ActorOptions, SyncActorOptions};
//!
//! /// Message type for [`counter`].
//! enum Message {
//!     /// Increase the counter, returning the current state.
//!     Add(RpcMessage<usize, usize>),
//!     /// Get the current state of the counter.
//!     Get(RpcMessage<(), usize>),
//! }
//!
//! /// Required to support RPC.
//! impl From<RpcMessage<usize, usize>> for Message {
//!     fn from(msg: RpcMessage<usize, usize>) -> Message {
//!         Message::Add(msg)
//!     }
//! }
//!
//! /// Required to support RPC.
//! impl From<RpcMessage<(), usize>> for Message {
//!     fn from(msg: RpcMessage<(), usize>) -> Message {
//!         Message::Get(msg)
//!     }
//! }
//!
//! /// Receiving synchronous actor of the RPC.
//! fn counter(mut ctx: SyncContext<Message>) -> Result<(), !> {
//!     // State of the counter.
//!     let mut count: usize = 0;
//!
//!     // Receive messages in a loop.
//!     while let Ok(msg) = ctx.receive_next() {
//!         match msg {
//!             Message::Add(RpcMessage { request, response }) => {
//!                 count += request;
//!                 // Send back the current state, ignoring any errors.
//!                 let _ = response.respond(count);
//!             },
//!             Message::Get(RpcMessage { response, .. }) => {
//!                 // Send back the current state, ignoring any errors.
//!                 let _ = response.respond(count);
//!             },
//!         }
//!     }
//!
//!     // And we're done.
//!     Ok(())
//! }
//!
//! /// Sending actor of the RPC.
//! async fn requester(mut ctx: actor::Context<!>, actor_ref: ActorRef<Message>) -> Result<(), !> {
//!     // Increase the counter by ten.
//!     // NOTE: do handle the errors correctly in practice, this is just an
//!     // example.
//!     let count = actor_ref.rpc(&mut ctx, 10).unwrap().await.unwrap();
//!     println!("Increased count to {}", count);
//!
//!     // Retrieve the current count.
//!     let count = actor_ref.rpc(&mut ctx, ()).unwrap().await.unwrap();
//! #   assert_eq!(count, 10);
//!     println!("Current count {}", count);
//!     Ok(())
//! }
//!
//! # fn main() -> Result<(), rt::Error> {
//! #    let mut runtime = Runtime::new()?;
//! #    let counter = counter as fn(_) -> _;
//! #    let options = SyncActorOptions::default();
//! #    let actor_ref = runtime.spawn_sync_actor(NoSupervisor, counter, (), options)?;
//! #    runtime.with_setup(|mut runtime_ref| {
//! #        let requester = requester as fn(_, _) -> _;
//! #        runtime_ref.spawn_local(NoSupervisor, requester, actor_ref, ActorOptions::default().mark_ready());
//! #        Ok(())
//! #    }).start()
//! # }
//! ```

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{self, Poll};

use crate::actor_ref::SendError;
use crate::inbox::oneshot::{self, Receiver, RecvError, Sender};
use crate::rt::Waker;

/// [`Future`] that resolves to a Remote Procedure Call (RPC) response.
#[derive(Debug)]
pub struct Rpc<Res> {
    recv: Receiver<Res>,
}

impl<Res> Rpc<Res> {
    /// Create a new RPC.
    pub(super) fn new<Req>(waker: Waker, request: Req) -> (RpcMessage<Req, Res>, Rpc<Res>) {
        let (send, recv) = oneshot::channel(waker);
        let response = RpcResponse { send };
        let msg = RpcMessage { request, response };
        let rpc = Rpc { recv };
        (msg, rpc)
    }
}

impl<Res> Future for Rpc<Res> {
    type Output = Result<Res, NoResponse>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        match self.get_mut().recv.try_recv() {
            Ok((response, _)) => Poll::Ready(Ok(response)),
            Err(RecvError::NoValue) => Poll::Pending,
            Err(RecvError::Disconnected) => Poll::Ready(Err(NoResponse)),
        }
    }
}

/// Error returned when an [`Rpc`] returned no response.
#[derive(Copy, Clone, Debug)]
pub struct NoResponse;

impl fmt::Display for NoResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no RPC response")
    }
}

impl Error for NoResponse {}

/// Message type that holds an RPC request.
///
/// It holds both the request (`Req`) and the way to respond [`RpcResponse`].
#[derive(Debug)]
pub struct RpcMessage<Req, Res> {
    /// The request object.
    pub request: Req,
    /// A way to [`respond`] to the call.
    ///
    /// [`respond`]: RpcResponse::respond
    pub response: RpcResponse<Res>,
}

/// Structure to respond to an [`Rpc`] request.
#[derive(Debug)]
pub struct RpcResponse<Res> {
    send: Sender<Res>,
}

impl<Res> RpcResponse<Res> {
    /// Respond to a RPC request.
    pub fn respond(self, response: Res) -> Result<(), SendError> {
        self.send.try_send(response).map_err(|_| SendError)
    }
}
