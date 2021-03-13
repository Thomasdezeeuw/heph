//! Tests for `TcpStream`.

#![cfg(feature = "test")]

use std::cmp::min;
use std::fs::{self, File};
use std::future::Future;
use std::io::{self, IoSlice, Read, Write};
use std::lazy::Lazy;
use std::net::{self, Shutdown, SocketAddr};
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{self, Poll};
use std::thread::sleep;
use std::time::Duration;

use heph::net::{TcpListener, TcpStream};
use heph::rt::ThreadLocal;
use heph::test::{init_local_actor, poll_actor};
use heph::{actor, Actor, ActorRef};

use crate::util::{
    any_local_address, expect_pending, expect_ready_ok, loop_expect_ready_ok, refused_address,
    run_actors, Stage,
};

const DATA: &[u8] = b"Hello world";
// Test files used in testing `send_file`.
const TEST_FILE0: &str = "./tests/data/hello_world";
const TEST_FILE1: &str = "./tests/data/lorem_ipsum";

// Contents of the test files.
const EXPECTED0: Lazy<Vec<u8>> =
    Lazy::new(|| fs::read(TEST_FILE0).expect("failed to read test file 0"));
const EXPECTED1: Lazy<Vec<u8>> =
    Lazy::new(|| fs::read(TEST_FILE1).expect("failed to read test file 0"));

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
    async fn actor(
        mut ctx: actor::Context<SocketAddr, ThreadLocal>,
        address: SocketAddr,
    ) -> io::Result<()> {
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
    let mut actor = Box::pin(actor);

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
fn connect() {
    static STAGE: Stage = Stage::new();

    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        STAGE.update(0);
        let connect = TcpStream::connect(&mut ctx, address)?;
        STAGE.update(1);
        wait_once().await;

        let stream = connect.await?;
        STAGE.update(2);
        wait_once().await;

        drop(stream);
        STAGE.update(3);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 1));

    // Once we accept the connecting the actor should be able to proceed.
    let (mut stream, _) = listener.accept().unwrap();
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 2));

    expect_ready_ok(STAGE.poll_till(Pin::as_mut(&mut actor), 3), ());

    let mut buf = [0; 2];
    assert_eq!(stream.read(&mut buf).unwrap(), 0);
    drop(stream);
}

#[test]
fn connect_connection_refused() {
    static STAGE: Stage = Stage::new();

    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
        STAGE.update(0);
        let connect = TcpStream::connect(&mut ctx, refused_address()).unwrap();
        STAGE.update(1);
        match connect.await {
            Ok(..) => panic!("unexpected success"),
            Err(err) => {
                assert_eq!(
                    err.kind(),
                    io::ErrorKind::ConnectionRefused,
                    "unexpected error: {:?}",
                    err
                );
            }
        }
        STAGE.update(2);
        Ok(())
    }

    let (actor, _) = init_local_actor(actor as fn(_) -> _, ()).unwrap();
    let mut actor = Box::pin(actor);
    expect_ready_ok(STAGE.poll_till(Pin::as_mut(&mut actor), 2), ());
}

#[test]
fn try_recv() {
    static STAGE: Stage = Stage::new();

    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        STAGE.update(0);
        let connect = TcpStream::connect(&mut ctx, address)?;
        STAGE.update(1);
        wait_once().await;

        let mut stream = connect.await?;
        STAGE.update(2);
        wait_once().await;

        let mut buf = Vec::with_capacity(128);
        match stream.try_recv(&mut buf) {
            Ok(n) => panic!("unexpected bytes: {:?} ({})", buf, n),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => return Err(err),
        }
        STAGE.update(3);

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

        STAGE.update(4);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 1));

    // Connected, but shouldn't yet read anything.
    let (mut stream, _) = listener.accept().unwrap();
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 3));

    // Should be able to read what we write.
    stream.write_all(&DATA).unwrap();
    sorta_flush(&mut stream);
    drop(stream);
    expect_ready_ok(STAGE.poll_till(Pin::as_mut(&mut actor), 4), ());
}

#[test]
fn recv() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
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
    let mut actor = Box::pin(actor);

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
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
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
    let mut actor = Box::pin(actor);

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
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
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
    let mut actor = Box::pin(actor);

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
fn recv_n_less_bytes() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
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
    let mut actor = Box::pin(actor);

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
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);
        stream.recv_n(&mut buf, 3 * DATA.len()).await?;
        assert_eq!(&buf[..DATA.len()], DATA);
        assert_eq!(&buf[DATA.len()..2 * DATA.len()], DATA);
        assert_eq!(&buf[2 * DATA.len()..], DATA);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

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
    static STAGE: Stage = Stage::new();

    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        STAGE.update(0);
        let connect = TcpStream::connect(&mut ctx, address)?;
        STAGE.update(1);
        wait_once().await;

        let mut stream = connect.await?;
        STAGE.update(2);
        wait_once().await;

        let n = stream.send(&DATA).await?;
        assert_eq!(n, DATA.len());
        STAGE.update(3);

        // Return pending once.
        wait_once().await;

        drop(stream);
        STAGE.update(4);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 1));

    // Once we accept the connecting the actor should be able to proceed.
    let (mut stream, _) = listener.accept().unwrap();
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 2));

    // Should send the bytes.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 3));

    let mut buf = [0; DATA.len() + 1];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, DATA.len());
    assert_eq!(&buf[..n], DATA);

    // Should drop the stream.
    expect_ready_ok(STAGE.poll_till(Pin::as_mut(&mut actor), 4), ());
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn send_all() {
    // A lot of data to get at least two write calls.
    const DATA: &[u8] = &[213; 40 * 1024];
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
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
    let mut actor = Box::pin(actor);

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

#[test]
fn send_vectored() {
    static STAGE: Stage = Stage::new();

    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        STAGE.update(0);
        let connect = TcpStream::connect(&mut ctx, address)?;
        STAGE.update(1);
        wait_once().await;

        let mut stream = connect.await?;
        STAGE.update(2);
        wait_once().await;

        let bufs = &mut [
            IoSlice::new(DATA),
            IoSlice::new(DATA),
            IoSlice::new(DATA),
            IoSlice::new(DATA),
        ];
        let n = stream.send_vectored(bufs).await?;
        assert_eq!(n, 4 * DATA.len());
        STAGE.update(3);

        // Return pending once.
        wait_once().await;

        drop(stream);
        STAGE.update(4);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 1));

    // Once we accept the connecting the actor should be able to proceed.
    let (mut stream, _) = listener.accept().unwrap();
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 2));

    // Should send the bytes.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 3));

    let mut buf = [0; (4 * DATA.len()) + 1];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 4 * DATA.len());
    for n in 0..4 {
        assert_eq!(&buf[n * DATA.len()..(n + 1) * DATA.len()], DATA);
    }

    // Should drop the stream.
    expect_ready_ok(STAGE.poll_till(Pin::as_mut(&mut actor), 4), ());
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn send_vectored_all() {
    // A lot of data to get at least two write calls.
    const DATA1: &[u8] = &[213; 40 * 1023];
    const DATA2: &[u8] = &[155; 30 * 1024];
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let bufs = &mut [IoSlice::new(DATA1), IoSlice::new(DATA2)];
        stream.send_vectored_all(bufs).await?;

        // Return pending once.
        wait_once().await;

        drop(stream);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

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
        if total >= DATA1.len() {
            // All in DATA2.
            let start = total - DATA1.len();
            assert_eq!(&buf[..n], &DATA2[start..start + n]);
        } else if total + n <= DATA1.len() {
            // All in DATA1.
            assert_eq!(&buf[..n], &DATA1[total..total + n]);
        } else {
            let m = min(total + n, DATA1.len());
            let n1 = m - total;
            assert_eq!(&buf[..n1], &DATA1[total..m]);
            let left = n - n1;
            assert_eq!(&buf[n1..n], &DATA2[..left]);
        }
        total += n;
    }
    assert_eq!(total, DATA1.len() + DATA2.len());

    // Should drop the stream.
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn recv_vectored() {
    static STAGE: Stage = Stage::new();

    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        STAGE.update(0);
        let connect = TcpStream::connect(&mut ctx, address)?;
        STAGE.update(1);
        wait_once().await;

        let mut stream = connect.await?;
        STAGE.update(2);
        wait_once().await;

        let mut buf1 = Vec::with_capacity(2 * DATA.len());
        let mut buf2 = Vec::with_capacity(2 * DATA.len() + 1);
        let bufs = [&mut buf1, &mut buf2];
        let n = stream.recv_vectored(bufs).await?;
        assert_eq!(n, 4 * DATA.len());
        assert_eq!(&buf1[..DATA.len()], DATA);
        assert_eq!(&buf1[DATA.len()..], DATA);
        assert_eq!(&buf2[..DATA.len()], DATA);
        assert_eq!(&buf2[DATA.len()..], DATA);
        STAGE.update(3);

        // Return pending once.
        wait_once().await;

        drop(stream);
        STAGE.update(4);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 1));

    // Once we accept the connecting the actor should be able to proceed.
    let (mut stream, _) = listener.accept().unwrap();
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 2));

    let bufs = &mut [
        IoSlice::new(DATA),
        IoSlice::new(DATA),
        IoSlice::new(DATA),
        IoSlice::new(DATA),
    ];
    stream.write_all_vectored(bufs).unwrap();

    // Should receive the bytes.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 3));

    // Should drop the stream.
    expect_ready_ok(STAGE.poll_till(Pin::as_mut(&mut actor), 4), ());
    let mut buf = [0; 2];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn recv_n_vectored_exact_amount() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf1 = Vec::with_capacity(DATA.len());
        let mut buf2 = Vec::with_capacity(DATA.len() + 1);
        let bufs = [&mut buf1, &mut buf2];
        stream.recv_n_vectored(bufs, 2 * DATA.len()).await?;
        assert_eq!(buf1, DATA);
        assert_eq!(buf2, DATA);

        // The stream is dropped so next we should read 0, which should cause an
        // `UnexpectedEof` error.
        buf1.clear();
        buf2.clear();
        let bufs = [&mut buf1, &mut buf2];
        match stream.recv_n_vectored(bufs, 10).await {
            Ok(()) => panic!("unexpected recv: {:?}", buf1),
            Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
            Err(err) => Err(err),
        }
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Connected, but shouldn't yet read anything.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let bufs = &mut [IoSlice::new(DATA), IoSlice::new(DATA)];
    stream.write_all_vectored(bufs).unwrap();
    sorta_flush(&mut stream);

    drop(stream);

    loop_expect_ready_ok(|| poll_actor(Pin::as_mut(&mut actor)), ());
}

#[test]
fn recv_n_vectored_more_bytes() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf1 = Vec::with_capacity(DATA.len());
        let mut buf2 = Vec::with_capacity(DATA.len() + 1);
        let bufs = [&mut buf1, &mut buf2];
        stream.recv_n_vectored(bufs, (2 * DATA.len()) - 3).await?;
        assert_eq!(buf1, DATA);
        assert_eq!(buf2, DATA);

        // The stream is dropped so next we should read 0, which should cause an
        // `UnexpectedEof` error.
        buf1.clear();
        buf2.clear();
        let bufs = [&mut buf1, &mut buf2];
        match stream.recv_n_vectored(bufs, 10).await {
            Ok(()) => panic!("unexpected recv: {:?}", buf1),
            Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
            Err(err) => Err(err),
        }
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Connected, but shouldn't yet read anything.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let bufs = &mut [IoSlice::new(DATA), IoSlice::new(DATA)];
    stream.write_all_vectored(bufs).unwrap();
    sorta_flush(&mut stream);

    drop(stream);

    loop_expect_ready_ok(|| poll_actor(Pin::as_mut(&mut actor)), ());
}

#[test]
fn recv_n_vectored_less_bytes() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf1 = Vec::with_capacity(DATA.len());
        let mut buf2 = Vec::with_capacity(DATA.len() + 1);
        let bufs = [&mut buf1, &mut buf2];
        match stream.recv_n_vectored(bufs, 2 * DATA.len()).await {
            Ok(()) => panic!("unexpected recv: {:?}", buf1),
            Err(ref err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
            Err(err) => Err(err),
        }
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    let (mut stream, _) = listener.accept().unwrap();

    // Connected, but shouldn't yet read anything.
    expect_pending(poll_actor(Pin::as_mut(&mut actor)));

    stream.write_all(DATA).unwrap();
    sorta_flush(&mut stream);

    drop(stream);

    loop_expect_ready_ok(|| poll_actor(Pin::as_mut(&mut actor)), ());
}

#[test]
fn recv_n_vectored_from_multiple_writes() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf1 = Vec::with_capacity(DATA.len());
        let mut buf2 = Vec::with_capacity(DATA.len());
        let mut buf3 = Vec::with_capacity(DATA.len() + 1);
        let bufs = [&mut buf1, &mut buf2, &mut buf3];
        stream.recv_n_vectored(bufs, 3 * DATA.len()).await?;
        assert_eq!(buf1, DATA);
        assert_eq!(buf2, DATA);
        assert_eq!(buf3, DATA);

        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

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
fn peek() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);

        let n = stream.peek(&mut buf).await?;
        assert_eq!(n, DATA.len());
        assert_eq!(&*buf, DATA);

        // We peeked the data above so we should receive the same data again.
        buf.clear();
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
    let mut actor = Box::pin(actor);

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
fn send_file() {
    // Should be able to send this many bytes in a single call.
    const LENGTH: usize = 128;

    async fn actor(
        mut ctx: actor::Context<!, ThreadLocal>,
        address: SocketAddr,
        path: &'static str,
    ) -> io::Result<()> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let length = min(metadata.len(), LENGTH as u64) as usize;
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
        let n = stream
            .send_file(&file, 0, NonZeroUsize::new(length))
            .await?;
        assert_eq!(n, length);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _, _) -> _;
    let (actor0, _) = init_local_actor(actor, (address, TEST_FILE0)).unwrap();
    let mut actor0 = Box::pin(actor0);
    expect_pending(poll_actor(Pin::as_mut(&mut actor0)));
    let (mut stream0, _) = listener.accept().unwrap();
    stream0.set_nonblocking(true).unwrap();

    let (actor1, _) = init_local_actor(actor, (address, TEST_FILE1)).unwrap();
    let mut actor1 = Box::pin(actor1);
    expect_pending(poll_actor(Pin::as_mut(&mut actor1)));
    let (mut stream1, _) = listener.accept().unwrap();
    stream1.set_nonblocking(true).unwrap();

    let mut expected0_offset = 0;
    let mut actor0_done = false;
    let expected1 = &EXPECTED1[..LENGTH];
    let mut expected1_offset = 0;
    let mut actor1_done = false;

    let mut buf = vec![0; LENGTH + 1];
    for _ in 0..20 {
        // NOTE: can't use `&&` as that short circuits.
        let done0 = send_file_check_actor(
            Pin::as_mut(&mut actor0),
            &mut actor0_done,
            &mut stream0,
            &EXPECTED0,
            &mut expected0_offset,
            &mut buf,
        );
        let done1 = send_file_check_actor(
            Pin::as_mut(&mut actor1),
            &mut actor1_done,
            &mut stream1,
            &expected1,
            &mut expected1_offset,
            &mut buf,
        );

        if done0 && done1 {
            break;
        }

        sleep(Duration::from_millis(10));
    }
}

#[test]
fn send_file_all() {
    const OFFSET: usize = 5;
    // Should be able to send this many bytes in a single call.
    const LENGTH: usize = 1 << 14; // 16kb.

    async fn actor(
        mut ctx: actor::Context<!, ThreadLocal>,
        address: SocketAddr,
        path: &'static str,
    ) -> io::Result<()> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let length = min(metadata.len(), LENGTH as u64) as usize;
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
        stream
            .send_file_all(&file, OFFSET, NonZeroUsize::new(length))
            .await?;
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _, _) -> _;
    let (actor0, _) = init_local_actor(actor, (address, TEST_FILE0)).unwrap();
    let mut actor0 = Box::pin(actor0);
    expect_pending(poll_actor(Pin::as_mut(&mut actor0)));
    let (mut stream0, _) = listener.accept().unwrap();
    stream0.set_nonblocking(true).unwrap();

    let (actor1, _) = init_local_actor(actor, (address, TEST_FILE1)).unwrap();
    let mut actor1 = Box::pin(actor1);
    expect_pending(poll_actor(Pin::as_mut(&mut actor1)));
    let (mut stream1, _) = listener.accept().unwrap();
    stream1.set_nonblocking(true).unwrap();

    let expected0 = &EXPECTED0;
    let mut expected0_offset = OFFSET;
    let mut actor0_done = false;
    let expected1 = &EXPECTED1[..OFFSET + LENGTH];
    let mut expected1_offset = OFFSET;
    let mut actor1_done = false;

    let mut buf = vec![0; LENGTH + 1];
    for _ in 0..20 {
        // NOTE: can't use `&&` as that short circuits.
        let done0 = send_file_check_actor(
            Pin::as_mut(&mut actor0),
            &mut actor0_done,
            &mut stream0,
            &expected0,
            &mut expected0_offset,
            &mut buf,
        );
        let done1 = send_file_check_actor(
            Pin::as_mut(&mut actor1),
            &mut actor1_done,
            &mut stream1,
            &expected1,
            &mut expected1_offset,
            &mut buf,
        );

        if done0 && done1 {
            break;
        }

        sleep(Duration::from_millis(10));
    }
}

#[test]
fn send_entire_file() {
    async fn actor(
        mut ctx: actor::Context<!, ThreadLocal>,
        address: SocketAddr,
        path: &'static str,
    ) -> io::Result<()> {
        let file = File::open(path)?;
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
        stream.send_entire_file(&file).await
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _, _) -> _;
    let (actor0, _) = init_local_actor(actor, (address, TEST_FILE0)).unwrap();
    let mut actor0 = Box::pin(actor0);
    expect_pending(poll_actor(Pin::as_mut(&mut actor0)));
    let (mut stream0, _) = listener.accept().unwrap();
    stream0.set_nonblocking(true).unwrap();

    let (actor1, _) = init_local_actor(actor, (address, TEST_FILE1)).unwrap();
    let mut actor1 = Box::pin(actor1);
    expect_pending(poll_actor(Pin::as_mut(&mut actor1)));
    let (mut stream1, _) = listener.accept().unwrap();
    stream1.set_nonblocking(true).unwrap();

    let mut expected0_offset = 0;
    let mut actor0_done = false;
    let mut expected1_offset = 0;
    let mut actor1_done = false;

    let mut buf = vec![0; 4096];
    for _ in 0..20 {
        // NOTE: can't use `&&` as that short circuits.
        let done0 = send_file_check_actor(
            Pin::as_mut(&mut actor0),
            &mut actor0_done,
            &mut stream0,
            &EXPECTED0,
            &mut expected0_offset,
            &mut buf,
        );
        let done1 = send_file_check_actor(
            Pin::as_mut(&mut actor1),
            &mut actor1_done,
            &mut stream1,
            &EXPECTED1,
            &mut expected1_offset,
            &mut buf,
        );

        if done0 && done1 {
            break;
        }

        sleep(Duration::from_millis(10));
    }
}

/// Returns `true` if `actor` send all `expected` bytes to `stream`.
#[track_caller]
fn send_file_check_actor<A: Actor<Error = io::Error>>(
    actor: Pin<&mut A>,
    actor_done: &mut bool,
    stream: &mut net::TcpStream,
    expected: &[u8],
    offset: &mut usize,
    buf: &mut Vec<u8>,
) -> bool {
    if !*actor_done {
        match poll_actor(actor) {
            Poll::Ready(Ok(())) => *actor_done = true,
            Poll::Ready(Err(err)) => panic!("unexpected error in actor: {}", err),
            Poll::Pending => {}
        }
    }

    if *offset != expected.len() {
        buf.resize(buf.capacity(), 0);
        match stream.read(&mut *buf) {
            Ok(0) => panic!("unexpected EOF"),
            Ok(n) => {
                assert_eq!(&expected[*offset..*offset + n], &buf[0..n]);
                *offset += n
            }
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => panic!("unexpected error reading: {}", err),
        }
    }

    // Done if we've read all expected bytes.
    *actor_done && *offset == expected.len()
}

#[test]
fn peek_vectored() {
    static STAGE: Stage = Stage::new();

    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        STAGE.update(0);
        let connect = TcpStream::connect(&mut ctx, address)?;
        STAGE.update(1);
        wait_once().await;

        let mut stream = connect.await?;
        STAGE.update(2);
        wait_once().await;

        let mut buf1 = Vec::with_capacity(2 * DATA.len());
        let mut buf2 = Vec::with_capacity(2 * DATA.len() + 1);
        let bufs = [&mut buf1, &mut buf2];
        let n = stream.peek_vectored(bufs).await?;
        assert_eq!(n, 4 * DATA.len());
        assert_eq!(&buf1[..DATA.len()], DATA);
        assert_eq!(&buf1[DATA.len()..], DATA);
        assert_eq!(&buf2[..DATA.len()], DATA);
        assert_eq!(&buf2[DATA.len()..], DATA);
        STAGE.update(3);

        // We should receive the same data again after peeking.
        buf1.clear();
        buf2.clear();
        let bufs = [&mut buf1, &mut buf2];
        let n = stream.recv_vectored(bufs).await?;
        assert_eq!(n, 4 * DATA.len());
        assert_eq!(&buf1[..DATA.len()], DATA);
        assert_eq!(&buf1[DATA.len()..], DATA);
        assert_eq!(&buf2[..DATA.len()], DATA);
        assert_eq!(&buf2[DATA.len()..], DATA);
        STAGE.update(3);

        // Return pending once.
        wait_once().await;

        drop(stream);
        STAGE.update(4);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let (actor, _) = init_local_actor(actor as fn(_, _) -> _, address).unwrap();
    let mut actor = Box::pin(actor);

    // Stream should not yet be connected.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 1));

    // Once we accept the connecting the actor should be able to proceed.
    let (mut stream, _) = listener.accept().unwrap();
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 2));

    let bufs = &mut [
        IoSlice::new(DATA),
        IoSlice::new(DATA),
        IoSlice::new(DATA),
        IoSlice::new(DATA),
    ];
    stream.write_all_vectored(bufs).unwrap();

    // Should receive the bytes.
    expect_pending(STAGE.poll_till(Pin::as_mut(&mut actor), 3));

    // Should drop the stream.
    expect_ready_ok(STAGE.poll_till(Pin::as_mut(&mut actor), 4), ());
    let mut buf = [0; 2];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn shutdown_read() {
    async fn listener_actor(
        mut ctx: actor::Context<!, ThreadLocal>,
        actor_ref: ActorRef<SocketAddr>,
    ) {
        let mut listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();

        let address = listener.local_addr().unwrap();
        actor_ref.send(address).await.unwrap();

        let (stream, remote_address) = listener.accept().await.unwrap();
        let mut stream = stream.bind_to(&mut ctx).unwrap();
        assert!(remote_address.ip().is_loopback());

        // Shutting down the reading side of the peer should return 0 bytes
        // here.
        let mut buf = Vec::with_capacity(DATA.len() + 1);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, DATA.len());
        assert_eq!(buf, DATA);
    }

    async fn stream_actor(mut ctx: actor::Context<SocketAddr, ThreadLocal>) {
        let address = ctx.receive_next().await.unwrap();
        let mut stream = TcpStream::connect(&mut ctx, address)
            .unwrap()
            .await
            .unwrap();

        stream.shutdown(Shutdown::Read).unwrap();

        let mut buf = Vec::with_capacity(2);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, 0);

        stream.send_all(DATA).await.unwrap();
    }

    let stream_actor = stream_actor as fn(_) -> _;
    let (stream_actor, actor_ref) = init_local_actor(stream_actor, ()).unwrap();
    let stream_actor: Box<dyn Actor<Error = !>> = Box::new(stream_actor);

    let listener_actor = listener_actor as fn(_, _) -> _;
    let (listener_actor, _) = init_local_actor(listener_actor, actor_ref).unwrap();
    let listener_actor: Box<dyn Actor<Error = !>> = Box::new(listener_actor);

    run_actors(vec![listener_actor.into(), stream_actor.into()]);
}

#[test]
fn shutdown_write() {
    async fn listener_actor(
        mut ctx: actor::Context<!, ThreadLocal>,
        actor_ref: ActorRef<SocketAddr>,
    ) {
        let mut listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();

        let address = listener.local_addr().unwrap();
        actor_ref.send(address).await.unwrap();

        let (stream, remote_address) = listener.accept().await.unwrap();
        let mut stream = stream.bind_to(&mut ctx).unwrap();
        assert!(remote_address.ip().is_loopback());

        // Shutting down the writing side of the peer should return EOF here.
        let mut buf = Vec::with_capacity(2);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, 0);

        stream.send_all(DATA).await.unwrap();
    }

    async fn stream_actor(mut ctx: actor::Context<SocketAddr, ThreadLocal>) {
        let address = ctx.receive_next().await.unwrap();
        let mut stream = TcpStream::connect(&mut ctx, address)
            .unwrap()
            .await
            .unwrap();

        stream.shutdown(Shutdown::Write).unwrap();

        let err = stream.send(DATA).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);

        let mut buf = Vec::with_capacity(DATA.len() + 1);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, DATA.len());
        assert_eq!(buf, DATA);
    }

    let stream_actor = stream_actor as fn(_) -> _;
    let (stream_actor, actor_ref) = init_local_actor(stream_actor, ()).unwrap();
    let stream_actor: Box<dyn Actor<Error = !>> = Box::new(stream_actor);

    let listener_actor = listener_actor as fn(_, _) -> _;
    let (listener_actor, _) = init_local_actor(listener_actor, actor_ref).unwrap();
    let listener_actor: Box<dyn Actor<Error = !>> = Box::new(listener_actor);

    run_actors(vec![listener_actor.into(), stream_actor.into()]);
}

#[test]
fn shutdown_both() {
    async fn listener_actor(
        mut ctx: actor::Context<!, ThreadLocal>,
        actor_ref: ActorRef<SocketAddr>,
    ) {
        let mut listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();

        let address = listener.local_addr().unwrap();
        actor_ref.send(address).await.unwrap();

        let (stream, remote_address) = listener.accept().await.unwrap();
        let mut stream = stream.bind_to(&mut ctx).unwrap();
        assert!(remote_address.ip().is_loopback());

        let mut buf = Vec::with_capacity(2);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    }

    async fn stream_actor(mut ctx: actor::Context<SocketAddr, ThreadLocal>) {
        let address = ctx.receive_next().await.unwrap();
        let mut stream = TcpStream::connect(&mut ctx, address)
            .unwrap()
            .await
            .unwrap();

        stream.shutdown(Shutdown::Both).unwrap();

        let err = stream.send(DATA).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);

        let mut buf = Vec::with_capacity(2);
        let n = stream.recv(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    }

    let stream_actor = stream_actor as fn(_) -> _;
    let (stream_actor, actor_ref) = init_local_actor(stream_actor, ()).unwrap();
    let stream_actor: Box<dyn Actor<Error = !>> = Box::new(stream_actor);

    let listener_actor = listener_actor as fn(_, _) -> _;
    let (listener_actor, _) = init_local_actor(listener_actor, actor_ref).unwrap();
    let listener_actor: Box<dyn Actor<Error = !>> = Box::new(listener_actor);

    run_actors(vec![listener_actor.into(), stream_actor.into()]);
}
