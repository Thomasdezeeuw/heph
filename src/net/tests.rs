//! Tests related to `net` types.

use std::net::SocketAddr;
use std::pin::Pin;
use std::task::Poll;
use std::{io, net};

use pin_utils::pin_mut;

use crate::actor::{self, Actor, NewActor};
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
    NA: NewActor<Message = !, Argument = SocketAddr, Actor = A, Error = !>,
    A: Actor<Error = io::Error>,
{
    let echo_socket = net::UdpSocket::bind(local_address).unwrap();
    let address = echo_socket.local_addr().unwrap();

    let (actor, _) = init_actor(new_actor, address).unwrap();
    pin_mut!(actor);

    // Send the data, peeking should return pending.
    match poll_actor(Pin::as_mut(&mut actor)) {
        Poll::Ready(Ok(())) => unreachable!(),
        Poll::Ready(Err(err)) => panic!("unexpected error from actor: {:?}", err),
        Poll::Pending => {} // Ok.
    }

    let mut buf = [0; DATA.len() + 2];
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
            Poll::Pending => continue,
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
