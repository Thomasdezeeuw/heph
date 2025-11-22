//! Networking primitives.
//!
//! To create a new socket ([`AsyncFd`]) use the [`socket`] function, which
//! issues a non-blocking `socket(2)` call.
//!
//! [`AsyncFd`]: crate::fd::AsyncFd

mod tcp_server;

#[doc(inline)]
pub use a10::net::{
    socket, Accept, Connect, MultishotAccept, MultishotRecv, NoAddress, Recv, RecvFrom,
    RecvFromVectored, RecvN, RecvNVectored, RecvVectored, Send, SendAll, SendAllVectored, SendMsg,
    SendTo, SetSocketOption, Shutdown, Socket, SocketAddress, SocketOption,
};
pub use tcp_server::{ServerError, ServerMessage, TcpServer};
