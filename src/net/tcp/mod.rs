//! Transmission Control Protocol (TCP) related types.
//!
//! Three main types are provided:
//!
//!  * [`TcpListener`] listens for incoming connections.
//!  * [`TcpStream`] represents a single TCP connection.
//!  * [`TcpServer`] is an [`Actor`] that listens for incoming connections and
//!    starts a new actor for each.
//!
//! [`Actor`]: crate::actor::Actor

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

pub mod listener;
pub mod server;
pub mod stream;

#[doc(no_inline)]
pub use listener::TcpListener;
#[doc(no_inline)]
pub use server::TcpServer;
#[doc(no_inline)]
pub use stream::TcpStream;
