//! Tests related to `UdpSocket`.

use std::io::{self, IoSlice};
use std::net::SocketAddr;
use std::time::Duration;

use heph_rt::actor::{self, Actor, Bound, NewActor};
use heph_rt::actor_ref::{ActorRef, RpcMessage};
use heph_rt::net::udp::{UdpSocket, Unconnected};
use heph_rt::rt::{self, Runtime, RuntimeRef, ThreadLocal};
use heph_rt::spawn::ActorOptions;
use heph_rt::supervisor::NoSupervisor;
use heph_rt::test::{join, try_spawn_local, PanicSupervisor};

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
    let actor = reconnecting_actor as fn(_, _, _) -> _;
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
fn actor_bound() {
    type Message = RpcMessage<UdpSocket<Unconnected>, ()>;

    async fn actor1<RT>(mut ctx: actor::Context<!, RT>, actor_ref: ActorRef<Message>)
    where
        RT: rt::Access,
    {
        let mut socket = UdpSocket::bind(&mut ctx, any_local_address()).unwrap();
        let peer_address = socket.local_addr().unwrap();
        let _ = actor_ref.rpc(socket).await.unwrap();

        let mut socket = UdpSocket::bind(&mut ctx, any_local_address()).unwrap();
        socket.send_to(DATA, peer_address).await.unwrap();
    }

    async fn actor2<RT>(mut ctx: actor::Context<Message, RT>)
    where
        RT: rt::Access,
    {
        let msg = ctx.receive_next().await.unwrap();
        let mut socket = msg.request;
        socket.bind_to(&mut ctx).unwrap();
        msg.response.respond(()).unwrap();
        let mut buf = Vec::with_capacity(DATA.len() + 1);
        let (n, _) = socket.recv_from(&mut buf).await.unwrap();
        assert_eq!(buf, DATA);
        assert_eq!(n, DATA.len());
    }

    fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
        // Spawn thread-local actors.
        let actor_ref = runtime_ref.spawn_local(
            NoSupervisor,
            actor2 as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            actor1 as fn(_, _) -> _,
            actor_ref,
            ActorOptions::default(),
        );

        Ok(())
    }

    let mut runtime = Runtime::setup().build().unwrap();
    runtime.run_on_workers(setup).unwrap();

    // Spawn thread-safe actors.
    let actor_ref = runtime.spawn(
        NoSupervisor,
        actor2 as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        actor1 as fn(_, _) -> _,
        actor_ref,
        ActorOptions::default(),
    );

    runtime.start().unwrap();
}
