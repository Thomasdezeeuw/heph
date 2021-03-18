//! Tests related to `UdpSocket`.

#![cfg(feature = "test")]

use std::io::{self, IoSlice};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::Poll;
use std::thread::sleep;
use std::time::Duration;

use heph::actor::{self, Actor, NewActor};
use heph::net::UdpSocket;
use heph::rt::ThreadLocal;
use heph::test::{init_local_actor, poll_actor};

use crate::util::{any_local_address, any_local_ipv6_address};

const DATA: &[u8] = b"Hello world";
const DATAV: &[&[u8]] = &[b"Hello world!", b" ", b"From mars."];
const DATAV_LEN: usize = DATAV[0].len() + DATAV[1].len() + DATAV[2].len();

#[test]
fn unconnected_ipv4() {
    let new_actor = unconnected_udp_actor as fn(_, _) -> _;
    test(any_local_address(), new_actor)
}

#[test]
fn unconnected_ipv6() {
    let new_actor = unconnected_udp_actor as fn(_, _) -> _;
    test(any_local_ipv6_address(), new_actor)
}

#[test]
fn connected_ipv4() {
    let new_actor = connected_udp_actor as fn(_, _) -> _;
    test(any_local_address(), new_actor)
}

#[test]
fn connected_ipv6() {
    let new_actor = connected_udp_actor as fn(_, _) -> _;
    test(any_local_ipv6_address(), new_actor)
}

fn test<NA>(local_address: SocketAddr, new_actor: NA)
where
    NA: NewActor<Argument = SocketAddr, Error = !, RuntimeAccess = ThreadLocal>,
    <NA as NewActor>::Actor: Actor<Error = io::Error>,
{
    let echo_socket = std::net::UdpSocket::bind(local_address).unwrap();
    let address = echo_socket.local_addr().unwrap();

    let (actor, _) = init_local_actor(new_actor, address).unwrap();
    let mut actor = Box::pin(actor);

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
    mut ctx: actor::Context<!, ThreadLocal>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let mut socket = UdpSocket::bind(&mut ctx, local_address)?;
    assert_eq!(socket.local_addr().unwrap().ip(), local_address.ip());

    let bytes_written = socket.send_to(&DATA, peer_address).await?;
    assert_eq!(bytes_written, DATA.len());

    let mut buf = Vec::with_capacity(DATA.len() + 2);
    let (bytes_peeked, address) = socket.peek_from(&mut buf).await?;
    assert_eq!(bytes_peeked, DATA.len());
    assert_eq!(&buf[..bytes_peeked], &*DATA);
    assert_eq!(address, peer_address);

    buf.clear();
    let (bytes_read, address) = socket.recv_from(&mut buf).await?;
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..bytes_read], &*DATA);
    assert_eq!(address, peer_address);

    assert!(socket.take_error().unwrap().is_none());

    Ok(())
}

async fn connected_udp_actor(
    mut ctx: actor::Context<!, ThreadLocal>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let socket = UdpSocket::bind(&mut ctx, local_address)?;
    let mut socket = socket.connect(peer_address)?;
    assert_eq!(socket.local_addr().unwrap().ip(), local_address.ip());

    let bytes_written = socket.send(&DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    let mut buf = Vec::with_capacity(DATA.len() + 2);
    let bytes_peeked = socket.peek(&mut buf).await?;
    assert_eq!(bytes_peeked, DATA.len());
    assert_eq!(&buf[..bytes_peeked], &*DATA);

    buf.clear();
    let bytes_read = socket.recv(&mut buf).await?;
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..bytes_read], &*DATA);

    assert!(socket.take_error().unwrap().is_none());

    Ok(())
}

#[test]
fn reconnecting_ipv4() {
    test_reconnecting(any_local_address())
}

#[test]
fn reconnecting_ipv6() {
    test_reconnecting(any_local_ipv6_address())
}

fn test_reconnecting(local_address: SocketAddr) {
    let local_address = SocketAddr::new(local_address.ip(), 0);
    let socket1 = std::net::UdpSocket::bind(local_address).unwrap();
    let socket2 = std::net::UdpSocket::bind(local_address).unwrap();

    let address1 = socket1.local_addr().unwrap();
    let address2 = socket2.local_addr().unwrap();

    #[allow(trivial_casts)]
    let reconnecting_actor = reconnecting_actor as fn(_, _, _) -> _;
    let (actor, _) = init_local_actor(reconnecting_actor, (address1, address2)).unwrap();
    let mut actor = Box::pin(actor);

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
    mut ctx: actor::Context<!, ThreadLocal>,
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

#[test]
fn connected_vectored_io_ipv4() {
    let new_actor = connected_vectored_io_actor as fn(_, _) -> _;
    test_vectored_io(any_local_address(), new_actor)
}

#[test]
fn connected_vectored_io_ipv6() {
    let new_actor = connected_vectored_io_actor as fn(_, _) -> _;
    test_vectored_io(any_local_ipv6_address(), new_actor)
}

#[test]
fn unconnected_vectored_io_ipv4() {
    let new_actor = unconnected_vectored_io_actor as fn(_, _) -> _;
    test_vectored_io(any_local_address(), new_actor)
}

#[test]
fn unconnected_vectored_io_ipv6() {
    let new_actor = unconnected_vectored_io_actor as fn(_, _) -> _;
    test_vectored_io(any_local_ipv6_address(), new_actor)
}

async fn unconnected_vectored_io_actor(
    mut ctx: actor::Context<!, ThreadLocal>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let mut socket = UdpSocket::bind(&mut ctx, local_address)?;

    let bufs = &mut [
        IoSlice::new(DATAV[0]),
        IoSlice::new(DATAV[1]),
        IoSlice::new(DATAV[2]),
    ];
    let bytes_written = socket.send_to_vectored(bufs, peer_address).await?;
    assert_eq!(bytes_written, DATAV_LEN);

    let mut buf1 = Vec::with_capacity(DATAV[0].len());
    let mut buf2 = Vec::with_capacity(DATAV[1].len());
    let mut buf3 = Vec::with_capacity(DATAV[2].len() + 2);
    let mut bufs = [&mut buf1, &mut buf2, &mut buf3];
    let (bytes_peeked, address) = socket.peek_from_vectored(&mut bufs).await?;
    assert_eq!(bytes_peeked, DATAV_LEN);
    assert_eq!(buf1, DATAV[0]);
    assert_eq!(buf2, DATAV[1]);
    assert_eq!(buf3, DATAV[2]);
    assert_eq!(address, peer_address);

    buf1.clear();
    buf2.clear();
    buf3.clear();
    let mut bufs = [&mut buf1, &mut buf2, &mut buf3];
    let (bytes_read, address) = socket.recv_from_vectored(&mut bufs).await?;
    assert_eq!(bytes_read, DATAV_LEN);
    assert_eq!(buf1, DATAV[0]);
    assert_eq!(buf2, DATAV[1]);
    assert_eq!(buf3, DATAV[2]);
    assert_eq!(address, peer_address);

    Ok(())
}

async fn connected_vectored_io_actor(
    mut ctx: actor::Context<!, ThreadLocal>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let socket = UdpSocket::bind(&mut ctx, local_address)?;
    let mut socket = socket.connect(peer_address)?;

    let bufs = &mut [
        IoSlice::new(DATAV[0]),
        IoSlice::new(DATAV[1]),
        IoSlice::new(DATAV[2]),
    ];
    let bytes_written = socket.send_vectored(bufs).await?;
    assert_eq!(bytes_written, DATAV_LEN);

    let mut buf1 = Vec::with_capacity(DATAV[0].len());
    let mut buf2 = Vec::with_capacity(DATAV[1].len());
    let mut buf3 = Vec::with_capacity(DATAV[2].len() + 2);
    let mut bufs = [&mut buf1, &mut buf2, &mut buf3];
    let bytes_peeked = socket.peek_vectored(&mut bufs).await?;
    assert_eq!(bytes_peeked, DATAV_LEN);
    assert_eq!(buf1, DATAV[0]);
    assert_eq!(buf2, DATAV[1]);
    assert_eq!(buf3, DATAV[2]);

    buf1.clear();
    buf2.clear();
    buf3.clear();
    let mut bufs = [&mut buf1, &mut buf2, &mut buf3];
    let bytes_read = socket.recv_vectored(&mut bufs).await?;
    assert_eq!(bytes_read, DATAV_LEN);
    assert_eq!(buf1, DATAV[0]);
    assert_eq!(buf2, DATAV[1]);
    assert_eq!(buf3, DATAV[2]);

    Ok(())
}

fn test_vectored_io<NA>(local_address: SocketAddr, new_actor: NA)
where
    NA: NewActor<Argument = SocketAddr, Error = !, RuntimeAccess = ThreadLocal>,
    <NA as NewActor>::Actor: Actor<Error = io::Error>,
{
    let echo_socket = std::net::UdpSocket::bind(local_address).unwrap();
    let address = echo_socket.local_addr().unwrap();

    let (actor, _) = init_local_actor(new_actor, address).unwrap();
    let mut actor = Box::pin(actor);

    // Send the data.
    match poll_actor(Pin::as_mut(&mut actor)) {
        Poll::Ready(Ok(())) => unreachable!(),
        Poll::Ready(Err(err)) => panic!("unexpected error from actor: {:?}", err),
        Poll::Pending => {} // Ok.
    }

    let mut buf = [0; DATAV_LEN + 1];
    let (bytes_read, peer_address) = echo_socket.recv_from(&mut buf).unwrap();
    assert_eq!(bytes_read, DATAV_LEN);
    assert_read(&buf, DATAV);

    let bytes_written = echo_socket
        .send_to(&buf[..bytes_read], peer_address)
        .unwrap();
    assert_eq!(bytes_written, DATAV_LEN);

    // The reading.
    loop {
        match poll_actor(Pin::as_mut(&mut actor)) {
            Poll::Ready(Ok(())) => return, // Ok.
            Poll::Ready(Err(err)) => panic!("unexpected error from actor: {:?}", err),
            Poll::Pending => sleep(Duration::from_millis(1)),
        }
    }
}

fn assert_read(mut got: &[u8], expected: &[&[u8]]) {
    for expected in expected.iter().copied() {
        let len = expected.len();
        assert_eq!(&got[..len], expected);
        let (_, g) = got.split_at(len);
        got = g;
    }
}
