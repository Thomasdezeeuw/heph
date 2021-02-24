//! Types related the `ActorRef` Remote Procedure Call (RPC) mechanism.
//!
//! RPC is implemented by sending a [`RpcMessage`] to the actor, which contains
//! the request message and a [`RpcResponse`]. The `RpcResponse` allows the
//! receiving actor to send back a response to the sending actor.
//!
//! To support RPC the receiving actor needs to implement
//! [`From`]`<`[`RpcMessage`]`<Req, Res>>`, where `Req` is the type of the
//! request message and `Res` the type of the response. This can be done easily
//! by using the [`from_message`] macro. The RPC message can then be received
//! like any other message.
//!
//! The sending actor needs to call [`ActorRef::rpc`] with the correct request
//! type. That will return an [`Rpc`] [`Future`] which returns the response to
//! the call, or [`RpcError`] in case of an error.
//!
//! [`from_message`]: crate::from_message
//!
//! # Examples
//!
//! Using RPC to communicate with another actor.
//!
//! ```
//! # #![feature(never_type)]
//! #
//! use heph::{actor, rt, Runtime, ActorOptions};
//! use heph::actor_ref::{ActorRef, RpcMessage};
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
//! async fn counter(mut ctx: actor::Context<Add>) {
//!     // State of the counter.
//!     let mut count: usize = 0;
//!     // Receive a message like normal.
//!     while let Ok(Add(RpcMessage { request, response })) = ctx.receive_next().await {
//!         count += request;
//!         // Send back the current state, ignoring any errors.
//!         let _ = response.respond(count);
//!     }
//! }
//!
//! /// Sending actor of the RPC.
//! async fn requester(_: actor::Context<!>, actor_ref: ActorRef<Add>) {
//!     // Make the procedure call.
//!     let response = actor_ref.rpc(10).await;
//! #   assert!(response.is_ok());
//!     match response {
//!         // We got a response.
//!         Ok(count) => println!("Current count: {}", count),
//!         // Actor failed to respond.
//!         Err(err) => eprintln!("Counter didn't reply: {}", err),
//!     }
//! }
//!
//! # fn main() -> Result<(), rt::Error> {
//! #    let mut runtime = Runtime::new()?;
//! #    runtime.run_on_workers(|mut runtime_ref| -> Result<(), !> {
//! #        let counter = counter as fn(_) -> _;
//! #        let actor_ref = runtime_ref.spawn_local(NoSupervisor, counter, (), ActorOptions::default());
//! #
//! #        let requester = requester as fn(_, _) -> _;
//! #        runtime_ref.spawn_local(NoSupervisor, requester, actor_ref, ActorOptions::default());
//! #        Ok(())
//! #    })?;
//! #    runtime.start()
//! # }
//! ```
//!
//! Supporting multiple procedure within the same actor is possible by making
//! the message an `enum` as the example below shows. Furthermore synchronous
//! actors are supported.
//!
// FIXME: doesn't stop on CI.
//! ```ignore
//! # #![feature(never_type)]
//! #
//! use heph::actor::{self, SyncContext};
//! use heph::actor_ref::{ActorRef, RpcMessage};
//! use heph::from_message;
//! use heph::rt::{self, Runtime, ActorOptions, SyncActorOptions};
//! use heph::supervisor::NoSupervisor;
//!
//! /// Message type for [`counter`].
//! enum Message {
//!     /// Increase the counter, returning the current state.
//!     Add(RpcMessage<usize, usize>),
//!     /// Get the current state of the counter.
//!     Get(RpcMessage<(), usize>),
//! }
//!
//! // Implement the `From` trait for `Message`.
//! from_message!(Message::Add(usize) -> usize);
//! from_message!(Message::Get(()) -> usize);
//!
//! /// Receiving synchronous actor of the RPC.
//! fn counter(mut ctx: SyncContext<Message>) {
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
//! }
//!
//! /// Sending actor of the RPC.
//! async fn requester(_: actor::Context<!>, actor_ref: ActorRef<Message>) {
//!     // Increase the counter by ten.
//!     // NOTE: do handle the errors correctly in practice, this is just an
//!     // example.
//!     let count = actor_ref.rpc(10).await.unwrap();
//!     println!("Increased count to {}", count);
//!
//!     // Retrieve the current count.
//!     let count = actor_ref.rpc(()).await.unwrap();
//! #   assert_eq!(count, 10);
//!     println!("Current count {}", count);
//! }
//!
//! # fn main() -> Result<(), rt::Error> {
//! #    let mut runtime = Runtime::new()?;
//! #    let counter = counter as fn(_) -> _;
//! #    let options = SyncActorOptions::default();
//! #    let actor_ref = runtime.spawn_sync_actor(NoSupervisor, counter, (), options)?;
//! #    runtime.run_on_workers(move |mut runtime_ref| -> Result<(), !> {
//! #        let requester = requester as fn(_, _) -> _;
//! #        runtime_ref.spawn_local(NoSupervisor, requester, actor_ref, ActorOptions::default());
//! #        Ok(())
//! #    })?;
//! #    runtime.start()
//! # }
//! ```

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{self, Poll};

use inbox::oneshot::{new_oneshot, RecvOnce, Sender};

use crate::actor_ref::{ActorRef, SendError, SendValue};

/// [`Future`] that resolves to a Remote Procedure Call (RPC) response.
///
/// Created by [`ActorRef::rpc`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Rpc<'r, 'fut, M, Res> {
    send: Option<SendValue<'r, 'fut, M>>,
    recv: RecvOnce<Res>,
}

impl<'r, 'fut, M, Res> Rpc<'r, 'fut, M, Res>
where
    'r: 'fut,
{
    /// Create a new RPC.
    pub(super) fn new<Req>(actor_ref: &'r ActorRef<M>, request: Req) -> Rpc<'r, 'fut, M, Res>
    where
        M: From<RpcMessage<Req, Res>>,
    {
        let (sender, receiver) = new_oneshot();
        let response = RpcResponse { sender };
        let msg = RpcMessage { request, response };
        let send = actor_ref.send(msg);
        Rpc {
            send: Some(send),
            recv: receiver.recv_once(),
        }
    }
}

impl<'r, 'fut, M, Res> Future for Rpc<'r, 'fut, M, Res> {
    type Output = Result<Res, RpcError>;

    #[track_caller]
    fn poll(mut self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // Safety: we're not moving `send` so this is safe.
        let send = unsafe { self.as_mut().map_unchecked_mut(|s| &mut s.send) }.as_pin_mut();
        if let Some(send) = send {
            match send.poll(ctx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err.into())),
                Poll::Pending => return Poll::Pending,
            }
            // Don't take this branch again.
            // Safety: we're not moving `send` so this is safe.
            unsafe { self.as_mut().map_unchecked_mut(|s| &mut s.send) }.set(None);
        }

        // Safety: we're not moving `recv` so this is safe.
        match unsafe { self.map_unchecked_mut(|s| &mut s.recv) }.poll(ctx) {
            Poll::Ready(Some(response)) => Poll::Ready(Ok(response)),
            Poll::Ready(None) => Poll::Ready(Err(RpcError::NoResponse)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Error returned by [`Rpc`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RpcError {
    /// Same error as [`SendError`].
    SendError,
    /// Returned when the other side returned no response.
    NoResponse,
}

impl From<SendError> for RpcError {
    fn from(_: SendError) -> RpcError {
        RpcError::SendError
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcError::SendError => SendError.fmt(f),
            RpcError::NoResponse => f.write_str("no RPC response"),
        }
    }
}

impl Error for RpcError {}

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

impl<Req, Res> RpcMessage<Req, Res> {
    /// Convenience method to handle a `Req`uest and return a `Res`ponse.
    ///
    /// The function `f` is called with [`self.request`], the response returned by
    /// the function `f` is than returned to the request maker via
    /// [`self.response.respond`].
    ///
    /// [`self.request`]: RpcMessage::request
    /// [`self.response.respond`]: RpcResponse::respond
    ///
    /// # Notes
    ///
    /// If the receiving end is [no longer connected] the function `f` is not
    /// called and `Ok(())` is returned instead.
    ///
    /// [no longer connected]: RpcResponse::is_connected
    pub fn handle<F>(self, f: F) -> Result<(), SendError>
    where
        F: FnOnce(Req) -> Res,
    {
        if self.response.is_connected() {
            let response = f(self.request);
            self.response.respond(response)
        } else {
            // If the receiving actor is no longer waiting we can skip the
            // request.
            Ok(())
        }
    }
}

/// Structure to respond to an [`Rpc`] request.
#[derive(Debug)]
pub struct RpcResponse<Res> {
    sender: Sender<Res>,
}

impl<Res> RpcResponse<Res> {
    /// Respond to a RPC request.
    pub fn respond(self, response: Res) -> Result<(), SendError> {
        self.sender.try_send(response).map_err(|_| SendError)
    }

    /// Returns `false` if the receiving side is disconnected.
    ///
    /// # Notes
    ///
    /// If this method returns `true` it doesn't mean that `respond` will
    /// succeed. In fact the moment this function returns a result it could
    /// already be invalid.
    pub fn is_connected(&self) -> bool {
        self.sender.is_connected()
    }
}
