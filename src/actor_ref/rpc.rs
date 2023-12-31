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
//! use heph::actor;
//! use heph::actor_ref::{ActorRef, RpcMessage};
//! use heph_rt::{self as rt, ThreadLocal};
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
//! async fn counter(mut ctx: actor::Context<Add, ThreadLocal>) {
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
//! async fn requester(_: actor::Context<!, ThreadLocal>, actor_ref: ActorRef<Add>) {
//!     // Make the procedure call.
//!     let response = actor_ref.rpc(10).await;
//! #   assert!(response.is_ok());
//!     match response {
//!         // We got a response.
//!         Ok(count) => println!("Current count: {count}"),
//!         // Actor failed to respond.
//!         Err(err) => eprintln!("Counter didn't reply: {err}"),
//!     }
//! }
//!
//! # fn main() -> Result<(), rt::Error> {
//! #    use heph::actor::actor_fn;
//! #    use heph::supervisor::NoSupervisor;
//! #    use heph_rt::Runtime;
//! #    use heph_rt::spawn::ActorOptions;
//! #    let mut runtime = Runtime::new()?;
//! #    runtime.run_on_workers(|mut runtime_ref| -> Result<(), !> {
//! #        let actor_ref = runtime_ref.spawn_local(NoSupervisor, actor_fn(counter), (), ActorOptions::default());
//! #        runtime_ref.spawn_local(NoSupervisor, actor_fn(requester), actor_ref, ActorOptions::default());
//! #        Ok(())
//! #    })?;
//! #    runtime.start()
//! # }
//! ```
//!
//! Supporting multiple procedures within the same actor is possible by making
//! the message an `enum` as the example below shows. Furthermore this example
//! shows that synchronous actors are supported as well.
//!
//! ```
//! # #![feature(never_type)]
//! #
//! use heph::actor_ref::{ActorRef, RpcMessage};
//! use heph::{actor, from_message, sync};
//! use heph_rt::{self as rt, ThreadLocal};
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
//! fn counter<RT>(mut ctx: sync::Context<Message, RT>) {
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
//!             Message::Get(RpcMessage { request: (), response }) => {
//!                 // Send back the current state, ignoring any errors.
//!                 let _ = response.respond(count);
//!             },
//!         }
//!     }
//! }
//!
//! /// Sending actor of the RPC.
//! async fn requester(_: actor::Context<!, ThreadLocal>, actor_ref: ActorRef<Message>) {
//!     // Increase the counter by ten.
//!     // NOTE: do handle the errors correctly in practice, this is just an
//!     // example.
//!     let count = actor_ref.rpc(10).await.unwrap();
//!     println!("Increased count to {count}");
//!
//!     // Retrieve the current count.
//!     let count = actor_ref.rpc(()).await.unwrap();
//! #   assert_eq!(count, 10);
//!     println!("Current count {count}");
//! }
//!
//! # fn main() -> Result<(), rt::Error> {
//! #    use heph::actor::actor_fn;
//! #    use heph::supervisor::NoSupervisor;
//! #    use heph_rt::Runtime;
//! #    use heph_rt::spawn::{ActorOptions, SyncActorOptions};
//! #
//! #    let mut runtime = Runtime::new()?;
//! #    let options = SyncActorOptions::default();
//! #    let actor_ref = runtime.spawn_sync_actor(NoSupervisor, actor_fn(counter), (), options)?;
//! #    runtime.run_on_workers(move |mut runtime_ref| -> Result<(), !> {
//! #        runtime_ref.spawn_local(NoSupervisor, actor_fn(requester), actor_ref, ActorOptions::default());
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

use heph_inbox::oneshot::{new_oneshot, RecvOnce, Sender};

use crate::actor_ref::{ActorRef, SendError, SendValue};

/// [`Future`] that resolves to a Remote Procedure Call (RPC) response.
///
/// Created by [`ActorRef::rpc`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Rpc<'r, M, Res> {
    send: Option<SendValue<'r, M>>,
    recv: RecvOnce<Res>,
}

impl<'r, M, Res> Rpc<'r, M, Res> {
    /// Create a new RPC.
    pub(super) fn new<Req>(actor_ref: &'r ActorRef<M>, request: Req) -> Rpc<'r, M, Res>
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

impl<'r, M, Res> Future for Rpc<'r, M, Res> {
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
    /// Returned when the other side disconnected without returning a response.
    ///
    /// # Notes
    ///
    /// Disconnects can not always be detected, consider this error informative
    /// rather than depending on it.
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
    /// The function `f` is called with [`self.request`] and the returned future
    /// is awaited, the response returned by the future is than returned to the
    /// requester via [`self.response.respond`].
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
    pub async fn handle<F, Fut>(self, f: F) -> Result<(), SendError>
    where
        F: FnOnce(Req) -> Fut,
        Fut: Future<Output = Res>,
    {
        if self.response.is_connected() {
            let response = f(self.request).await;
            self.response.respond(response)
        } else {
            // If the receiving actor is no longer waiting we can skip the
            // request.
            Ok(())
        }
    }

    /// Convenience method to handle a `Req`uest and return a `Res`ponse.
    ///
    /// This is similar to [`handle`], but allows `f` to be failable.
    ///
    /// [`handle`]: RpcMessage::handle
    pub async fn try_handle<F, Fut, E>(self, f: F) -> Result<Result<(), SendError>, E>
    where
        F: FnOnce(Req) -> Fut,
        Fut: Future<Output = Result<Res, E>>,
    {
        if self.response.is_connected() {
            let response = f(self.request).await?;
            Ok(self.response.respond(response))
        } else {
            // If the receiving actor is no longer waiting we can skip the
            // request.
            Ok(Ok(()))
        }
    }
}

/// Structure to respond to an [`Rpc`] request.
#[derive(Debug)]
pub struct RpcResponse<Res> {
    sender: Sender<Res>,
}

impl<Res> RpcResponse<Res> {
    /// Respond to the RPC request.
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
