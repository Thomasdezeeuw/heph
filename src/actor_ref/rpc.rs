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
