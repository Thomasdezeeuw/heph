//! Tests related to `UdpSocket`.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use heph::actor::{self, actor_fn, Actor, NewActor};
use heph_rt::net::udp::{UdpSocket, Unconnected};
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{block_on_local_actor, join, try_spawn_local, PanicSupervisor};
use heph_rt::ThreadLocal;

use crate::util::{any_local_address, any_local_ipv6_address};

const DATA: &[u8] = b"Hello world";
const DATAV: &[&[u8]] = &[b"Hello world!", b" ", b"From mars."];
const DATAV_LEN: usize = DATAV[0].len() + DATAV[1].len() + DATAV[2].len();

#[test]
fn unconnected_ipv4() {
    test(any_local_address(), actor_fn(unconnected_udp_actor))
}

#[test]
fn unconnected_ipv6() {
    test(any_local_ipv6_address(), actor_fn(unconnected_udp_actor))
}

#[test]
fn connected_ipv4() {
    test(any_local_address(), actor_fn(connected_udp_actor))
}

#[test]
fn connected_ipv6() {
    test(any_local_ipv6_address(), actor_fn(connected_udp_actor))
}

fn test<NA>(local_address: SocketAddr, new_actor: NA)
where
    NA: NewActor<Argument = SocketAddr, Error = !, RuntimeAccess = ThreadLocal> + Send + 'static,
    <NA as NewActor>::Actor: Actor<Error = io::Error>,
    NA::Message: Send,
{
    let echo_socket = std::net::UdpSocket::bind(local_address).unwrap();
    let address = echo_socket.local_addr().unwrap();

    let actor_ref =
        try_spawn_local(PanicSupervisor, new_actor, address, ActorOptions::default()).unwrap();

    let mut buf = [0; DATA.len() + 1];
    let (bytes_read, peer_address) = echo_socket.recv_from(&mut buf).unwrap();
    assert_eq!(bytes_read, DATA.len());
    assert_eq!(&buf[..bytes_read], &*DATA);

    let bytes_written = echo_socket
        .send_to(&buf[..bytes_read], peer_address)
        .unwrap();
    assert_eq!(bytes_written, DATA.len());

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

async fn unconnected_udp_actor(
    ctx: actor::Context<!, ThreadLocal>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let socket = UdpSocket::bind(ctx.runtime_ref(), local_address).await?;
    assert_eq!(socket.local_addr().unwrap().ip(), local_address.ip());

    let (_, bytes_written) = socket.send_to(DATA, peer_address).await?;
    assert_eq!(bytes_written, DATA.len());

    let (mut buf, address) = socket.peek_from(Vec::with_capacity(DATA.len() + 2)).await?;
    assert_eq!(buf, DATA);
    assert_eq!(address, peer_address);

    buf.clear();
    let (buf, address) = socket.recv_from(buf).await?;
    assert_eq!(buf, DATA);
    assert_eq!(address, peer_address);

    assert!(socket.take_error().unwrap().is_none());

    Ok(())
}

async fn connected_udp_actor(
    ctx: actor::Context<!, ThreadLocal>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let socket = UdpSocket::bind(ctx.runtime_ref(), local_address).await?;
    let socket = socket.connect(peer_address).await?;
    assert_eq!(socket.local_addr().unwrap().ip(), local_address.ip());

    let (_, bytes_written) = socket.send(DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    let mut buf = socket.peek(Vec::with_capacity(DATA.len() + 2)).await?;
    assert_eq!(buf, DATA);

    buf.clear();
    let buf = socket.recv(buf).await?;
    assert_eq!(buf, DATA);

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

    let actor = actor_fn(reconnecting_actor);
    let args = (address1, address2);
    let actor_ref = try_spawn_local(PanicSupervisor, actor, args, ActorOptions::default()).unwrap();

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

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

async fn reconnecting_actor(
    ctx: actor::Context<!, ThreadLocal>,
    peer_address1: SocketAddr,
    peer_address2: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address1.ip(), 0);
    let socket = UdpSocket::bind(ctx.runtime_ref(), local_address).await?;
    let socket = socket.connect(peer_address1).await?;

    let (_, bytes_written) = socket.send(DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    let socket = socket.connect(peer_address1).await?;
    let (_, bytes_written) = socket.send(DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    let socket = socket.connect(peer_address2).await?;
    let (_, bytes_written) = socket.send(DATA).await?;
    assert_eq!(bytes_written, DATA.len());

    assert!(socket.take_error().unwrap().is_none());

    Ok(())
}

#[test]
fn connected_vectored_io_ipv4() {
    let new_actor = actor_fn(connected_vectored_io_actor);
    test_vectored_io(any_local_address(), new_actor)
}

#[test]
fn connected_vectored_io_ipv6() {
    let new_actor = actor_fn(connected_vectored_io_actor);
    test_vectored_io(any_local_ipv6_address(), new_actor)
}

#[test]
fn unconnected_vectored_io_ipv4() {
    let new_actor = actor_fn(unconnected_vectored_io_actor);
    test_vectored_io(any_local_address(), new_actor)
}

#[test]
fn unconnected_vectored_io_ipv6() {
    let new_actor = actor_fn(unconnected_vectored_io_actor);
    test_vectored_io(any_local_ipv6_address(), new_actor)
}

async fn unconnected_vectored_io_actor(
    ctx: actor::Context<!, ThreadLocal>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let socket = UdpSocket::bind(ctx.runtime_ref(), local_address).await?;

    let bufs = [DATAV[0], DATAV[1], DATAV[2]];
    let (_, bytes_written) = socket.send_to_vectored(bufs, peer_address).await?;
    assert_eq!(bytes_written, DATAV_LEN);

    let bufs = [
        Vec::with_capacity(DATAV[0].len()),
        Vec::with_capacity(DATAV[1].len()),
        Vec::with_capacity(DATAV[2].len() + 2),
    ];
    let (mut bufs, address) = socket.peek_from_vectored(bufs).await?;
    assert_eq!(bufs[0], DATAV[0]);
    assert_eq!(bufs[1], DATAV[1]);
    assert_eq!(bufs[2], DATAV[2]);
    assert_eq!(address, peer_address);

    for buf in bufs.iter_mut() {
        buf.clear();
    }
    let (bufs, address) = socket.recv_from_vectored(bufs).await?;
    assert_eq!(bufs[0], DATAV[0]);
    assert_eq!(bufs[1], DATAV[1]);
    assert_eq!(bufs[2], DATAV[2]);
    assert_eq!(address, peer_address);

    Ok(())
}

async fn connected_vectored_io_actor(
    ctx: actor::Context<!, ThreadLocal>,
    peer_address: SocketAddr,
) -> io::Result<()> {
    let local_address = SocketAddr::new(peer_address.ip(), 0);
    let socket = UdpSocket::bind(ctx.runtime_ref(), local_address).await?;
    let socket = socket.connect(peer_address).await?;

    let bufs = [DATAV[0], DATAV[1], DATAV[2]];
    let (_, bytes_written) = socket.send_vectored(bufs).await?;
    assert_eq!(bytes_written, DATAV_LEN);

    let bufs = [
        Vec::with_capacity(DATAV[0].len()),
        Vec::with_capacity(DATAV[1].len()),
        Vec::with_capacity(DATAV[2].len() + 2),
    ];
    let mut bufs = socket.peek_vectored(bufs).await?;
    assert_eq!(bufs[0], DATAV[0]);
    assert_eq!(bufs[1], DATAV[1]);
    assert_eq!(bufs[2], DATAV[2]);

    for buf in bufs.iter_mut() {
        buf.clear();
    }
    let bufs = socket.recv_vectored(bufs).await?;
    assert_eq!(bufs[0], DATAV[0]);
    assert_eq!(bufs[1], DATAV[1]);
    assert_eq!(bufs[2], DATAV[2]);

    Ok(())
}

fn test_vectored_io<NA>(local_address: SocketAddr, new_actor: NA)
where
    NA: NewActor<Argument = SocketAddr, Error = !, RuntimeAccess = ThreadLocal> + Send + 'static,
    <NA as NewActor>::Actor: Actor<Error = io::Error>,
    NA::Message: Send,
{
    let echo_socket = std::net::UdpSocket::bind(local_address).unwrap();
    let address = echo_socket.local_addr().unwrap();

    let actor_ref =
        try_spawn_local(PanicSupervisor, new_actor, address, ActorOptions::default()).unwrap();

    let mut buf = [0; DATAV_LEN + 1];
    let (bytes_read, peer_address) = echo_socket.recv_from(&mut buf).unwrap();
    assert_eq!(bytes_read, DATAV_LEN);
    assert_read(&buf, DATAV);

    let bytes_written = echo_socket
        .send_to(&buf[..bytes_read], peer_address)
        .unwrap();
    assert_eq!(bytes_written, DATAV_LEN);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

fn assert_read(mut got: &[u8], expected: &[&[u8]]) {
    for expected in expected.iter().copied() {
        let len = expected.len();
        assert_eq!(&got[..len], expected);
        let (_, g) = got.split_at(len);
        got = g;
    }
}

#[test]
fn socket_from_std() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
        let socket = std::net::UdpSocket::bind(any_local_address())?;
        let socket = UdpSocket::<Unconnected>::from_std(ctx.runtime_ref(), socket);
        let local_address = socket.local_addr()?;

        let peer = std::net::UdpSocket::bind(any_local_address())?;
        let peer_address = peer.local_addr()?;

        let (_, bytes_written) = socket.send_to(DATA, peer_address).await?;
        assert_eq!(bytes_written, DATA.len());

        let mut buf = vec![0; DATA.len() + 2];
        let (n, address) = peer.recv_from(&mut buf)?;
        assert_eq!(n, DATA.len());
        assert_eq!(&buf[..n], DATA);
        assert_eq!(address, local_address);

        Ok(())
    }

    block_on_local_actor(actor_fn(actor), ());
}
