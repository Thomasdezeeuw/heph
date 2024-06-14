//! Networking primitives.
//!
//! The network module support two types of protocols:
//!
//! * [Transmission Control Protocol] (TCP) module provides three main types:
//!   * A [TCP stream] between a local and a remote socket.
//!   * A [TCP listening socket], a socket used to listen for connections.
//!   * A [TCP server], listens for connections and starts a new actor for each.
//! * [User Datagram Protocol] (UDP) only provides a single socket type:
//!   * [`UdpSocket`].
//! * [Unix Domain Socket] (UDS) module provides three types:
//!   * A [Unix stream] between two socket.
//!   * A [Unix listening socket], a socket used to listen for connections.
//!   * A [Unix datagram socket].
//!
//! [Transmission Control Protocol]: crate::net::tcp
//! [TCP stream]: crate::net::TcpStream
//! [TCP listening socket]: crate::net::TcpListener
//! [TCP server]: crate::net::tcp::server
//! [User Datagram Protocol]: crate::net::udp
//! [Unix Domain Socket]: crate::net::uds
//! [Unix stream]: crate::net::UnixStream
//! [Unix listening socket]: crate::net::UnixListener
//! [Unix datagram socket]: crate::net::UnixDatagram

use std::mem::MaybeUninit;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::{fmt, io, ptr};

mod futures;
pub mod tcp;
pub mod udp;
pub mod uds;

#[doc(no_inline)]
pub use tcp::{TcpListener, TcpStream};
#[doc(no_inline)]
pub use udp::UdpSocket;
#[doc(no_inline)]
pub use uds::{UnixDatagram, UnixListener, UnixStream};

pub(crate) use futures::{
    Recv, RecvFrom, RecvFromVectored, RecvN, RecvNVectored, RecvVectored, Send, SendAll,
    SendAllVectored, SendTo, SendToVectored, SendVectored,
};

/// The unconnected mode of an [`UdpSocket`] or [`UnixDatagram`].
#[allow(missing_debug_implementations)]
#[allow(clippy::empty_enum)]
pub enum Unconnected {}

/// The connected mode of an [`UdpSocket`] or [`UnixDatagram`].
#[allow(missing_debug_implementations)]
#[allow(clippy::empty_enum)]
pub enum Connected {}

/// Convert a `socket2:::SockAddr` into a `std::net::SocketAddr`.
#[allow(clippy::needless_pass_by_value)]
fn convert_address(address: socket2::SockAddr) -> io::Result<SocketAddr> {
    match address.as_socket() {
        Some(address) => Ok(address),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid address family (not IPv4 or IPv6)",
        )),
    }
}

// TODO: merge this into socket2 in some form.
#[derive(Copy, Clone)]
pub(crate) union SockAddr {
    ip: libc::sockaddr,
    ipv4: libc::sockaddr_in,
    ipv6: libc::sockaddr_in6,
}

impl From<SocketAddr> for SockAddr {
    fn from(addr: SocketAddr) -> SockAddr {
        match addr {
            SocketAddr::V4(addr) => addr.into(),
            SocketAddr::V6(addr) => addr.into(),
        }
    }
}

impl From<SocketAddrV4> for SockAddr {
    #[allow(clippy::cast_possible_truncation)]
    fn from(addr: SocketAddrV4) -> SockAddr {
        SockAddr {
            ipv4: libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: addr.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(addr.ip().octets()),
                },
                sin_zero: Default::default(),
            },
        }
    }
}

impl From<SocketAddrV6> for SockAddr {
    #[allow(clippy::cast_possible_truncation)]
    fn from(addr: SocketAddrV6) -> SockAddr {
        SockAddr {
            ipv6: libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: addr.port().to_be(),
                sin6_addr: libc::in6_addr {
                    s6_addr: addr.ip().octets(),
                },
                sin6_flowinfo: addr.flowinfo(),
                sin6_scope_id: addr.scope_id(),
            },
        }
    }
}

impl From<SockAddr> for SocketAddr {
    #[allow(clippy::cast_lossless)]
    fn from(addr: SockAddr) -> SocketAddr {
        match unsafe { addr.ip.sa_family as _ } {
            libc::AF_INET => {
                let addr = unsafe { addr.ipv4 };
                let ip = Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes());
                let port = u16::from_be(addr.sin_port);
                SocketAddr::V4(SocketAddrV4::new(ip, port))
            }
            libc::AF_INET6 => {
                let addr = unsafe { addr.ipv6 };
                let ip = Ipv6Addr::from(addr.sin6_addr.s6_addr);
                let port = u16::from_be(addr.sin6_port);
                SocketAddr::V6(SocketAddrV6::new(
                    ip,
                    port,
                    addr.sin6_flowinfo,
                    addr.sin6_scope_id,
                ))
            }
            _ => unreachable!(),
        }
    }
}

impl a10::net::SocketAddress for SockAddr {
    #[allow(clippy::cast_lossless)]
    unsafe fn as_ptr(&self) -> (*const libc::sockaddr, libc::socklen_t) {
        match unsafe { self.ip.sa_family as _ } {
            libc::AF_INET => self.ipv4.as_ptr(),
            libc::AF_INET6 => self.ipv6.as_ptr(),
            _ => unreachable!(),
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    unsafe fn as_mut_ptr(this: &mut MaybeUninit<Self>) -> (*mut libc::sockaddr, libc::socklen_t) {
        (
            ptr::addr_of_mut!(*this.as_mut_ptr()).cast(),
            size_of::<SockAddr>() as _,
        )
    }

    unsafe fn init(this: MaybeUninit<Self>, length: libc::socklen_t) -> Self {
        debug_assert!(length as usize >= size_of::<libc::sa_family_t>());
        // SAFETY: caller must initialise the address.
        this.assume_init()
    }
}

impl fmt::Debug for SockAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        SocketAddr::from(*self).fmt(f)
    }
}
