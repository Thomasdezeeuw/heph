//! Transmission Control Protocol (TCP) related types.
//!
//! Three main types are provided:
//!
//!  * [`TcpListener`] listens for incoming connections.
//!  * [`TcpStream`] represents a single TCP connection.
//!  * [`tcp::Server`] is an [`Actor`] that listens for incoming connections and
//!    starts a new actor for each.
//!
//! [`tcp::Server`]: crate::net::tcp::Server
//! [`Actor`]: crate::actor::Actor
//! [`tcp::NewListener`]: crate::net::tcp::NewListener

// Used in the `server` module.
macro_rules! syscall {
    ($fn: ident ( $($arg: expr),* $(,)* ) ) => {{
        let res = unsafe { libc::$fn($($arg, )*) };
        if res == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(res)
        }
    }};
}

mod server;

pub mod listener;
pub mod stream;

pub use listener::TcpListener;
pub use server::{Server, ServerError, ServerMessage, ServerSetup};
pub use stream::TcpStream;
