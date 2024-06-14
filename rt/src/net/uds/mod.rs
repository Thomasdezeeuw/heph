//! Unix Domain Socket (UDS) or Inter-Process Communication (IPC) related types.
//!
//! Three main types are provided:
//!
//!  * [`UnixListener`] listens for incoming Unix connections.
//!  * [`UnixStream`] represents a Unix stream socket.
//!  * [`UnixDatagram`] represents a Unix datagram socket.

use std::mem::MaybeUninit;
use std::path::Path;
use std::{io, ptr};

use socket2::SockAddr;

pub mod datagram;
pub mod listener;
pub mod stream;

#[doc(no_inline)]
pub use datagram::UnixDatagram;
#[doc(no_inline)]
pub use listener::UnixListener;
#[doc(no_inline)]
pub use stream::UnixStream;

/// Unix socket address.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnixAddr {
    /// NOTE: must always be of type `AF_UNIX`.
    inner: SockAddr,
}

impl UnixAddr {
    /// Create a `UnixAddr` from `path`.
    pub fn from_pathname<P>(path: P) -> io::Result<UnixAddr>
    where
        P: AsRef<Path>,
    {
        SockAddr::unix(path.as_ref()).map(|a| UnixAddr { inner: a })
    }

    /// Returns the contents of this address if it is a pathname address.
    pub fn as_pathname(&self) -> Option<&Path> {
        self.inner.as_pathname()
    }
}

/// **Not part of the API, do not use**.
#[doc(hidden)]
impl a10::net::SocketAddress for UnixAddr {
    unsafe fn as_ptr(&self) -> (*const libc::sockaddr, libc::socklen_t) {
        (self.inner.as_ptr(), self.inner.len())
    }

    #[allow(clippy::cast_possible_truncation)]
    unsafe fn as_mut_ptr(this: &mut MaybeUninit<Self>) -> (*mut libc::sockaddr, libc::socklen_t) {
        (
            ptr::addr_of_mut!((*this.as_mut_ptr()).inner).cast(),
            size_of::<libc::sockaddr_storage>() as _,
        )
    }

    unsafe fn init(this: MaybeUninit<Self>, length: libc::socklen_t) -> Self {
        debug_assert!(length as usize >= size_of::<libc::sa_family_t>());
        // SAFETY: caller must initialise the address.
        let mut this = this.assume_init();
        this.inner.set_length(length);
        debug_assert!(this.inner.is_unix());
        this
    }
}
