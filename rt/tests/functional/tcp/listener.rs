use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use heph::actor::{self, Bound};
use heph::supervisor::NoSupervisor;
use heph::util::next;
use heph::{ActorOptions, ActorRef};
use heph_rt::net::{TcpListener, TcpStream};
use heph_rt::rt::{self, Runtime, RuntimeRef, ThreadLocal};
use heph_rt::test::{init_local_actor, join_many, poll_actor, try_spawn_local};

use crate::util::{any_local_address, any_local_ipv6_address, pending_once};

#[test]
fn local_addr() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
        let address = "127.0.0.1:12345".parse().unwrap();
        let mut listener = TcpListener::bind(&mut ctx, address).unwrap();
        assert_eq!(listener.local_addr().unwrap(), address);
        drop(listener);

        let address = "[::1]:12345".parse().unwrap();
        let mut listener = TcpListener::bind(&mut ctx, address).unwrap();
        assert_eq!(listener.local_addr().unwrap(), address);
    }

    let actor = actor as fn(_) -> _;
    let (actor, _) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn local_addr_port_zero() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
        let address = any_local_address();
        let mut listener = TcpListener::bind(&mut ctx, address).unwrap();
        let got = listener.local_addr().unwrap();
        assert_eq!(got.ip(), address.ip());
        assert!(got.port() != 0);
        drop(listener);

        let address = any_local_ipv6_address();
        let mut listener = TcpListener::bind(&mut ctx, address).unwrap();
        let got = listener.local_addr().unwrap();
        assert_eq!(got.ip(), address.ip());
        assert!(got.port() != 0);
    }

    let actor = actor as fn(_) -> _;
    let (actor, _) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
}

#[test]
fn ttl() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) {
        let mut listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();

        let initial = listener.ttl().unwrap();
        let expected = initial + 10;
        listener.set_ttl(expected).unwrap();
        assert_eq!(listener.ttl().unwrap(), expected);
    }

    let actor = actor as fn(_) -> _;
    let (actor, _) = init_local_actor(actor, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(poll_actor(Pin::as_mut(&mut actor)), Poll::Ready(Ok(())));
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
fn try_accept() {
    async fn listener_actor<M>(
        mut ctx: actor::Context<M, ThreadLocal>,
        actor_ref: ActorRef<SocketAddr>,
    ) {
        let mut listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();

        let address = listener.local_addr().unwrap();
        actor_ref.send(address).await.unwrap();

        assert_eq!(
            listener.try_accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        let mut stream = loop {
            pending_once().await;
            if let Ok((stream, remote_address)) = listener.try_accept() {
                assert!(remote_address.ip().is_loopback());
                break stream.bind_to(&mut ctx).unwrap();
            }
        };

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
fn accept() {
    async fn listener_actor<M>(
        mut ctx: actor::Context<M, ThreadLocal>,
        actor_ref: ActorRef<SocketAddr>,
    ) {
        let mut listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();

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
        let mut listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();

        let address = listener.local_addr().unwrap();
        actor_ref.send(address).await.unwrap();

        let mut incoming = listener.incoming();
        let (stream, remote_address) = next(&mut incoming).await.unwrap().unwrap();
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
fn actor_bound() {
    async fn listener_actor1<RT>(mut ctx: actor::Context<!, RT>, actor_ref: ActorRef<TcpListener>)
    where
        RT: rt::Access,
    {
        let listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();
        actor_ref.send(listener).await.unwrap();
    }

    async fn listener_actor2<RT>(
        mut ctx: actor::Context<TcpListener, RT>,
        actor_ref: ActorRef<SocketAddr>,
    ) where
        RT: rt::Access,
    {
        let mut listener = ctx.receive_next().await.unwrap();
        listener.bind_to(&mut ctx).unwrap();

        let address = listener.local_addr().unwrap();
        actor_ref.send(address).await.unwrap();

        let (stream, remote_address) = listener.accept().await.unwrap();
        let mut stream = stream.bind_to(&mut ctx).unwrap();
        assert!(remote_address.ip().is_loopback());

        stream.bind_to(&mut ctx).unwrap();

        let mut buf = Vec::with_capacity(DATA.len() + 1);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, DATA.len());
        assert_eq!(buf, DATA);
    }

    fn setup(mut runtime_ref: RuntimeRef) -> Result<(), !> {
        // Spawn thread-local actors.
        let stream_ref = runtime_ref.spawn_local(
            NoSupervisor,
            stream_actor as fn(_) -> _,
            (),
            ActorOptions::default(),
        );
        let listener_ref = runtime_ref.spawn_local(
            NoSupervisor,
            listener_actor2 as fn(_, _) -> _,
            stream_ref,
            ActorOptions::default(),
        );
        let _ = runtime_ref.spawn_local(
            NoSupervisor,
            listener_actor1 as fn(_, _) -> _,
            listener_ref,
            ActorOptions::default(),
        );
        Ok(())
    }

    let mut runtime = Runtime::setup().build().unwrap();
    runtime.run_on_workers(setup).unwrap();

    // Spawn thread-safe actors.
    let stream_ref = runtime.spawn(
        NoSupervisor,
        stream_actor as fn(_) -> _,
        (),
        ActorOptions::default(),
    );
    let listener_ref = runtime.spawn(
        NoSupervisor,
        listener_actor2 as fn(_, _) -> _,
        stream_ref,
        ActorOptions::default(),
    );
    let _ = runtime.spawn(
        NoSupervisor,
        listener_actor1 as fn(_, _) -> _,
        listener_ref,
        ActorOptions::default(),
    );

    runtime.start().unwrap();
}
