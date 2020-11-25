//! Tests for `TcpStream`.

#![cfg(feature = "test")]

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

use crate::util::{
    any_local_address, expect_pending, expect_ready_ok, loop_expect_ready_ok, Stage,
};

const DATA: &[u8] = b"Hello world";

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

        let expected_address = ctx.receive_next().await.unwrap();
        assert_eq!(local_address, expected_address);

        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, actor_ref) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));
    // Still not connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, address) = listener.accept().unwrap();
    let _ = actor_ref.try_send(address);

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
    static STAGE: Stage = Stage::new();

    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        STAGE.update(0);
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);
        match stream.try_recv(&mut buf) {
            Ok(n) => panic!("unexpected bytes: {:?} ({})", buf, n),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => return Err(err),
        }
        STAGE.update(1);

        // Return pending once.
        wait_once().await;

        limited_loop! {
            match stream.try_recv(&mut buf) {
                Ok(n) => {
                    assert_eq!(n, DATA.len());
                    assert_eq!(&*buf, DATA);
                    break;
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                    sleep(Duration::from_millis(1));
                    continue;
                }
                Err(err) => return Err(err),
            }
        }

        // The stream is dropped, so we should read 0.
        buf.clear();
        limited_loop! {
            match stream.try_recv(&mut buf) {
                Ok(n) => {
                    assert_eq!(n, 0);
                    break;
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                    sleep(Duration::from_millis(1));
                    continue;
                },
                Err(err) => return Err(err),
            }
        }

        STAGE.update(2);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 0));

    // Connected, but shouldn't yet read anything.
    let (mut stream, _) = listener.accept().unwrap();
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 1));

    // Should be able to read what we write.
    stream.write_all(&DATA).unwrap();
    sorta_flush(&mut stream);
    drop(stream);
    expect_ready_ok(STAGE.poll_till(Pin::as_mut(&mut actor), 2), ());
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

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
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

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
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

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
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

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
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

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
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

#[test]
fn send() {
    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let n = stream.send(&DATA).await?;
        assert_eq!(n, DATA.len());

        // Return pending once.
        wait_once().await;

        drop(stream);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Should send the bytes.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let mut buf = [0; DATA.len() + 1];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, DATA.len());
    assert_eq!(&buf[..n], DATA);

    // Should drop the stream.
    expect_ready_ok(poll_actor(Pin::as_mut(&mut actor)), ());
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn send_all() {
    // A lot of data to get at least two write calls.
    const DATA: &[u8] = &[213; 40 * 1024];
    async fn actor(mut ctx: actor::Context<!>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        stream.send_all(DATA).await?;

        // Return pending once.
        wait_once().await;

        drop(stream);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    pin_mut!(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    let mut buf = [0; 8 * 1024];
    let mut total = 0;
    let mut dont_poll = false;
    loop {
        if !dont_poll {
            // Should send the bytes.
            match poll_actor(Pin::as_mut(&mut actor)) {
                Poll::Pending => {}
                Poll::Ready(Ok(())) => dont_poll = true,
                Poll::Ready(Err(err)) => panic!("unexpected error: {}", err),
            }
        }

        let n = stream.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        assert_eq!(&buf[..n], &DATA[total..total + n]);
        total += n;
    }
    assert_eq!(total, DATA.len());

    // Should drop the stream.
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

// TODO: test:
// * TcpStream::shutdown.
