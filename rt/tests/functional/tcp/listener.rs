use std::net::SocketAddr;
use std::time::Duration;

use heph::supervisor::NoSupervisor;
use heph::{actor, ActorRef};
use heph_rt::net::{TcpListener, TcpStream};
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{join, join_many, try_spawn_local};
use heph_rt::util::next;
use heph_rt::{self as rt, ThreadLocal};

use crate::util::{any_local_address, any_local_ipv6_address};

#[test]
fn local_addr() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let address = "127.0.0.1:12345".parse().unwrap();
        let mut listener = TcpListener::bind(ctx.runtime_ref(), address).await.unwrap();
        assert_eq!(listener.local_addr().unwrap(), address);
        drop(listener);

        let address = "[::1]:12345".parse().unwrap();
        let mut listener = TcpListener::bind(ctx.runtime_ref(), address).await.unwrap();
        assert_eq!(listener.local_addr().unwrap(), address);
    }

    let actor = actor as fn(_) -> _;
    let actor_ref = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn local_addr_port_zero() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let address = any_local_address();
        let mut listener = TcpListener::bind(ctx.runtime_ref(), address).await.unwrap();
        let got = listener.local_addr().unwrap();
        assert_eq!(got.ip(), address.ip());
        assert!(got.port() != 0);
        drop(listener);

        let address = any_local_ipv6_address();
        let mut listener = TcpListener::bind(ctx.runtime_ref(), address).await.unwrap();
        let got = listener.local_addr().unwrap();
        assert_eq!(got.ip(), address.ip());
        assert!(got.port() != 0);
    }

    let actor = actor as fn(_) -> _;
    let actor_ref = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn ttl() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let mut listener = TcpListener::bind(ctx.runtime_ref(), any_local_address())
            .await
            .unwrap();

        let initial = listener.ttl().unwrap();
        let expected = initial + 10;
        listener.set_ttl(expected).unwrap();
        assert_eq!(listener.ttl().unwrap(), expected);
    }

    let actor = actor as fn(_) -> _;
    let actor_ref = try_spawn_local(NoSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

const DATA: &[u8] = b"Hello world";

async fn stream_actor<RT>(mut ctx: actor::Context<SocketAddr, RT>)
where
    RT: rt::Access,
{
    let address = ctx.receive_next().await.unwrap();
    let mut stream = TcpStream::connect(&mut ctx, address)
        .unwrap()
        .await
        .unwrap();

    let n = stream.send(DATA).await.unwrap();
    assert_eq!(n, DATA.len());
}

#[test]
fn accept() {
    async fn listener_actor<M>(
        mut ctx: actor::Context<M, ThreadLocal>,
        actor_ref: ActorRef<SocketAddr>,
    ) {
        let mut listener = TcpListener::bind(ctx.runtime_ref(), any_local_address())
            .await
            .unwrap();

        let address = listener.local_addr().unwrap();
        actor_ref.send(address).await.unwrap();

        let (stream, remote_address) = listener.accept().await.unwrap();
        let mut stream = stream.bind_to(&mut ctx).unwrap();
        assert!(remote_address.ip().is_loopback());

        let mut buf = Vec::with_capacity(DATA.len() + 1);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, DATA.len());
        assert_eq!(buf, DATA);
    }

    let stream_actor = stream_actor as fn(_) -> _;
    let stream_ref =
        try_spawn_local(NoSupervisor, stream_actor, (), ActorOptions::default()).unwrap();

    let listener_actor = listener_actor as fn(_, _) -> _;
    let s_ref = stream_ref.clone();
    let listener_ref =
        try_spawn_local(NoSupervisor, listener_actor, s_ref, ActorOptions::default()).unwrap();

    join_many(&[stream_ref, listener_ref], Duration::from_secs(1)).unwrap();
}

#[test]
fn incoming() {
    async fn listener_actor<M>(
        mut ctx: actor::Context<M, ThreadLocal>,
        actor_ref: ActorRef<SocketAddr>,
    ) {
        let mut listener = TcpListener::bind(ctx.runtime_ref(), any_local_address())
            .await
            .unwrap();

        let address = listener.local_addr().unwrap();
        actor_ref.send(address).await.unwrap();

        let mut incoming = listener.incoming();
        let stream = next(&mut incoming).await.unwrap().unwrap();
        let mut stream = stream.bind_to(&mut ctx).unwrap();

        let mut buf = Vec::with_capacity(DATA.len() + 1);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, DATA.len());
        assert_eq!(buf, DATA);
    }

    let stream_actor = stream_actor as fn(_) -> _;
    let stream_ref =
        try_spawn_local(NoSupervisor, stream_actor, (), ActorOptions::default()).unwrap();

    let listener_actor = listener_actor as fn(_, _) -> _;
    let s_ref = stream_ref.clone();
    let listener_ref =
        try_spawn_local(NoSupervisor, listener_actor, s_ref, ActorOptions::default()).unwrap();

    join_many(&[stream_ref, listener_ref], Duration::from_secs(1)).unwrap();
}
