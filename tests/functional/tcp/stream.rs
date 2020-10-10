use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{self, SocketAddr};
use std::pin::Pin;
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use futures_util::pin_mut;

use heph::actor;
use heph::net::TcpStream;
use heph::test::{init_local_actor, poll_actor};

use crate::util::{expect_pending, expect_ready_ok, loop_expect_ready_ok};

const DATA: &[u8] = b"Hello world";

/// Returns: "127.0.0.1:0".
fn any_port() -> SocketAddr {
    SocketAddr::V4(net::SocketAddrV4::new(net::Ipv4Addr::LOCALHOST, 0))
}

/// Sort of flush a `TcpStream`.
fn sorta_flush(stream: &mut net::TcpStream) {
    // TCP sockets don't have a concept of "flushing". But some of the tests
    // depend on the data in the socket being send before proceeding. So we
    // nudge the OS by setting `TCP_NODELAY` and sleeping for a while.
    stream.set_nodelay(true).unwrap();
    sleep(Duration::from_millis(50));
}

/// Return [`Poll::Pending`] once.
const fn wait_once() -> WaitOnce {
    WaitOnce(false)
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
struct WaitOnce(bool);

impl Future for WaitOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

#[test]
fn smoke() {
    async fn actor(mut ctx: actor::Context<SocketAddr>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        assert_eq!(stream.peer_addr().unwrap(), address);
        let local_address = stream.local_addr().unwrap();
        assert!(local_address.ip().is_loopback());

        let ttl = stream.ttl().unwrap();
        stream.set_ttl(ttl + 1).unwrap();
        assert_eq!(stream.ttl().unwrap(), ttl + 1);

        let nodelay = stream.nodelay().unwrap();
        stream.set_nodelay(!nodelay).unwrap();
        assert_eq!(stream.nodelay().unwrap(), !nodelay);

        let keepalive = stream.keepalive().unwrap();
        stream.set_keepalive(!keepalive).unwrap();
        assert_eq!(stream.keepalive().unwrap(), !keepalive);

        assert!(stream.take_error().unwrap().is_none());

        let expected_address = ctx.receive_next().await;
        assert_eq!(local_address, expected_address);

        Ok(())
    }

    let listener = net::TcpListener::bind(any_port()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, actor_ref) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));
    // Still not connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, address) = listener.accept().unwrap();
    let _ = actor_ref.send(address);

    // Now we expect the stream to be connected and ready.
    expect_ready_ok(poll_actor(Pin::as_mut(&mut actor)), ());

    // Now the actor is done it should have dropped the stream should have been
    // dropped.
    let mut buf = [0; 8];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn try_recv() {
    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);
        match stream.try_recv(&mut buf) {
            Ok(n) => panic!("unexpected bytes: {:?} ({})", buf, n),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => return Err(err),
        }

        // Return pending once.
        wait_once().await;

        let n = loop {
            match stream.try_recv(&mut buf) {
                Ok(n) => break n,
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => continue,
                Err(err) => return Err(err),
            }
        };
        assert_eq!(n, DATA.len());
        assert_eq!(&*buf, DATA);

        // The stream is dropped, so we should read 0.
        buf.clear();
        loop {
            match stream.try_recv(&mut buf) {
                Ok(n) => {
                    assert_eq!(n, 0);
                    return Ok(());
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => continue,
                Err(err) => return Err(err),
            }
        }
    }

    let listener = net::TcpListener::bind(any_port()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Connected, but shouldn't yet read anything.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    stream.write_all(&DATA).unwrap();
    sorta_flush(&mut stream);
    drop(stream);

    expect_ready_ok(poll_actor(Pin::as_mut(&mut actor)), ());
}

#[test]
fn recv() {
    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);

        let n = stream.recv(&mut buf).await?;
        assert_eq!(n, DATA.len());
        assert_eq!(&*buf, DATA);

        // The stream is dropped so next we should read 0.
        buf.clear();
        assert_eq!(stream.recv(&mut buf).await?, 0);

        Ok(())
    }

    let listener = net::TcpListener::bind(any_port()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Connected, but shouldn't yet read anything.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    stream.write_all(&DATA).unwrap();
    sorta_flush(&mut stream);

    drop(stream);

    loop_expect_ready_ok(|| poll_actor(Pin::as_mut(&mut actor)), ());
}

#[test]
fn recv_n_read_exact_amount() {
    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);
        stream.recv_n(&mut buf, DATA.len()).await?;
        assert_eq!(buf.len(), DATA.len());
        assert_eq!(&*buf, DATA);

        // The stream is dropped so next we should read 0, which should cause an
        // `UnexpectedEof` error.
        buf.clear();
        match stream.recv_n(&mut buf, 10).await {
            Ok(()) => panic!("unexpected recv: {:?}", buf),
            Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
            Err(err) => Err(err),
        }
    }

    let listener = net::TcpListener::bind(any_port()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Connected, but shouldn't yet read anything.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    stream.write_all(&DATA).unwrap();
    sorta_flush(&mut stream);

    drop(stream);

    loop_expect_ready_ok(|| poll_actor(Pin::as_mut(&mut actor)), ());
}

#[test]
fn recv_n_read_more_bytes() {
    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);

        let want_n = DATA.len() - 2;
        stream.recv_n(&mut buf, want_n).await?;
        // We should still receive all data, not limiting ourselves to `want_n`
        // bytes.
        assert_eq!(buf.len(), DATA.len());
        assert_eq!(&*buf, DATA);

        // The stream is dropped so next we should read 0, which should cause an
        // `UnexpectedEof` error.
        buf.clear();
        match stream.recv_n(&mut buf, 10).await {
            Ok(()) => panic!("unexpected recv: {:?}", buf),
            Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
            Err(err) => Err(err),
        }
    }

    let listener = net::TcpListener::bind(any_port()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Connected, but shouldn't yet read anything.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    stream.write_all(&DATA).unwrap();
    sorta_flush(&mut stream);

    drop(stream);

    loop_expect_ready_ok(|| poll_actor(Pin::as_mut(&mut actor)), ());
}

#[test]
fn recv_n_less() {
    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);

        let want_n = 2 * DATA.len();
        match stream.recv_n(&mut buf, want_n).await {
            Ok(()) => panic!("unexpected recv: {:?}", buf),
            Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
            Err(err) => Err(err),
        }
    }

    let listener = net::TcpListener::bind(any_port()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Connected, but shouldn't yet read anything.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    stream.write_all(&DATA).unwrap();
    sorta_flush(&mut stream);

    drop(stream);

    loop_expect_ready_ok(|| poll_actor(Pin::as_mut(&mut actor)), ());
}

#[test]
fn recv_n_from_multiple_writes() {
    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);

        let want_n = 3 * DATA.len();
        match stream.recv_n(&mut buf, want_n).await {
            Ok(()) => Ok(()),
            Err(err) => Err(err),
        }
    }

    let listener = net::TcpListener::bind(any_port()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    for _ in 0..3 {
        expect_pending(poll_actor(Pin::as_mut(&mut actor)));

        stream.write_all(&DATA).unwrap();
        sorta_flush(&mut stream);
    }

    drop(stream);

    loop_expect_ready_ok(|| poll_actor(Pin::as_mut(&mut actor)), ());
}
