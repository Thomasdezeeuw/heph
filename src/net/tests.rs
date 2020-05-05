//! Tests related to `net` types.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::Poll;
use std::thread::sleep;
use std::time::Duration;

use futures_util::pin_mut;

use crate::actor::{self, context, Actor, NewActor};
use crate::net::UdpSocket;
use crate::test::{init_actor, poll_actor};

const DATA: &[u8] = b"Hello world";

#[test]
fn unconnected_udp_socket_ipv4() {
    #[allow(trivial_casts)]
    let new_actor = unconnected_udp_actor as fn(_, _) -> _;
    test_udp_socket(any_local_address(), new_actor)
}

#[test]
fn unconnected_udp_socket_ipv6() {
    #[allow(trivial_casts)]
    let new_actor = unconnected_udp_actor as fn(_, _) -> _;
    test_udp_socket(any_local_ipv6_address(), new_actor)
}

#[test]
fn connected_udp_socket_ipv4() {
    #[allow(trivial_casts)]
    let new_actor = connected_udp_actor as fn(_, _) -> _;
    test_udp_socket(any_local_address(), new_actor)
}

#[test]
fn connected_udp_socket_ipv6() {
    #[allow(trivial_casts)]
    let new_actor = connected_udp_actor as fn(_, _) -> _;
    test_udp_socket(any_local_ipv6_address(), new_actor)
}

fn test_udp_socket<NA, A>(local_address: SocketAddr, new_actor: NA)
where
    NA: NewActor<
        Message = !,
        Argument = SocketAddr,
        Actor = A,
        Error = !,
        Context = context::ThreadLocal,
    >,
    A: Actor<Error = io::Error>,
{
    let echo_socket = std::net::UdpSocket::bind(local_address).unwrap();
    let address = echo_socket.local_addr().unwrap();

    let (actor, _) = init_actor(new_actor, address).unwrap();
    pin_mut!(actor);

    // Send the data, peeking should return pending.
    match poll_actor(Pin::as_mut(&mut actor)) {
        Poll::Ready(Ok(())) => unreachable!(),
        Poll::Ready(Err(err)) => panic!("unexpected error from actor: {:?}", err),
        Poll::Pending => {} // Ok.
    }

    let mut buf = [0; DATA.len() + 1];
    let (bytes_read, peer_address) = echo_socket.recv_from(&mut buf).unwrap();
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..bytes_read], &*DATA);

    let bytes_written = echo_socket
        .send_to(&buf[..bytes_read], peer_address)
        .unwrap();
    assert_eq!(bytes_written, DATA.len());

    // The peeking and reading.
    loop {
        match poll_actor(Pin::as_mut(&mut actor)) {
            Poll::Ready(Ok(())) => return, // Ok.
            Poll::Ready(Err(err)) => panic!("unexpected error from actor: {:?}", err),
            Poll::Pending => sleep(Duration::from_millis(1)),
        }
    }
}

async fn unconnected_udp_actor(
    mut ctx: actor::Context<!>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let mut socket = UdpSocket::bind(&mut ctx, local_address)?;

    let bytes_written = socket.send_to(&DATA, peer_address).await?;
    assert_eq!(bytes_written, DATA.len());

    let mut buf = [0; DATA.len() + 2];
    let (bytes_peeked, address) = socket.peek_from(&mut buf).await?;
    assert_eq!(bytes_peeked, DATA.len());
    assert_eq!(&buf[..bytes_peeked], &*DATA);
    assert_eq!(address, peer_address);

    let (bytes_read, address) = socket.recv_from(&mut buf).await?;
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..bytes_read], &*DATA);
    assert_eq!(address, peer_address);

    assert!(socket.take_error().unwrap().is_none());

    Ok(())
}

async fn connected_udp_actor(
    mut ctx: actor::Context<!>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let socket = UdpSocket::bind(&mut ctx, local_address)?;
    let mut socket = socket.connect(peer_address)?;

    let bytes_written = socket.send(&DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    let mut buf = [0; DATA.len() + 2];
    let bytes_peeked = socket.peek(&mut buf).await?;
    assert_eq!(bytes_peeked, DATA.len());
    assert_eq!(&buf[..bytes_peeked], &*DATA);

    let bytes_read = socket.recv(&mut buf).await?;
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..bytes_read], &*DATA);

    assert!(socket.take_error().unwrap().is_none());

    Ok(())
}

#[test]
fn reconnecting_udp_socket_ipv4() {
    test_reconnecting_udp_socket(any_local_address())
}

#[test]
fn reconnecting_udp_socket_ipv6() {
    test_reconnecting_udp_socket(any_local_ipv6_address())
}

fn test_reconnecting_udp_socket(local_address: SocketAddr) {
    let local_address = SocketAddr::new(local_address.ip(), 0);
    let socket1 = std::net::UdpSocket::bind(local_address).unwrap();
    let socket2 = std::net::UdpSocket::bind(local_address).unwrap();

    let address1 = socket1.local_addr().unwrap();
    let address2 = socket2.local_addr().unwrap();

    #[allow(trivial_casts)]
    let reconnecting_actor = reconnecting_actor as fn(_, _, _) -> _;
    let (actor, _) = init_actor(reconnecting_actor, (address1, address2)).unwrap();
    pin_mut!(actor);

    // Let the actor send all data.
    loop {
        match poll_actor(Pin::as_mut(&mut actor)) {
            Poll::Ready(Ok(())) => break,
            Poll::Ready(Err(err)) => panic!("unexpected error from actor: {:?}", err),
            Poll::Pending => sleep(Duration::from_millis(1)),
        }
    }

    let mut buf = [0; DATA.len() + 1];
    // The first socket should receive the data twice.
    let (bytes_read, peer_address) = socket1.recv_from(&mut buf).unwrap();
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..DATA.len()], &*DATA);
    let (bytes_read, peer_address1) = socket1.recv_from(&mut buf).unwrap();
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..DATA.len()], &*DATA);
    assert_eq!(peer_address, peer_address1);

    let (bytes_read, peer_address1) = socket2.recv_from(&mut buf).unwrap();
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..DATA.len()], &*DATA);
    assert_eq!(peer_address, peer_address1);
}

async fn reconnecting_actor(
    mut ctx: actor::Context<!>,
    peer_address1: SocketAddr,
    peer_address2: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address1.ip(), 0);
    let socket = UdpSocket::bind(&mut ctx, local_address)?;
    let mut socket = socket.connect(peer_address1)?;

    let bytes_written = socket.send(&DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    let mut socket = socket.connect(peer_address1)?;
    let bytes_written = socket.send(&DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    let mut socket = socket.connect(peer_address2)?;
    let bytes_written = socket.send(&DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    assert!(socket.take_error().unwrap().is_none());

    Ok(())
}

/// Bind to any IPv4 port on localhost.
pub fn any_local_address() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

/// Bind to any IPv6 port on localhost.
pub fn any_local_ipv6_address() -> SocketAddr {
    "[::1]:0".parse().unwrap()
}
