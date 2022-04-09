//! Tests for `TcpStream`.

use std::cmp::min;
use std::fs::{self, File};
use std::io::{self, IoSlice, Read, Write};
use std::lazy::SyncLazy;
use std::net::{self, Shutdown, SocketAddr};
use std::num::NonZeroUsize;
use std::thread::sleep;
use std::time::Duration;

use heph::actor::{self, Bound};
use heph::actor_ref::{ActorRef, RpcMessage};
use heph::spawn::ActorOptions;
use heph::supervisor::NoSupervisor;
use heph_rt::net::{TcpListener, TcpStream};
use heph_rt::rt::{self, Runtime, RuntimeRef, ThreadLocal};
use heph_rt::test::{join, join_many, try_spawn_local, PanicSupervisor};

use crate::util::{any_local_address, refused_address};

const DATA: &[u8] = b"Hello world";
// Test files used in testing `send_file`.
const TEST_FILE0: &str = "./tests/data/hello_world";
const TEST_FILE1: &str = "./tests/data/lorem_ipsum";

// Contents of the test files.
static EXPECTED0: SyncLazy<Vec<u8>> =
    SyncLazy::new(|| fs::read(TEST_FILE0).expect("failed to read test file 0"));
static EXPECTED1: SyncLazy<Vec<u8>> =
    SyncLazy::new(|| fs::read(TEST_FILE1).expect("failed to read test file 0"));

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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, address) = listener.accept().unwrap();
    let _ = actor_ref.try_send(address);
    // Now the actor is done it should have dropped the stream should have been
    // dropped.
    let mut buf = [0; 8];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn connect() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let stream = TcpStream::connect(&mut ctx, address)?.await?;
        drop(stream);
        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    // Once we accept the connecting the actor should be able to proceed.
    let (mut stream, _) = listener.accept().unwrap();
    let mut buf = [0; 2];
    assert_eq!(stream.read(&mut buf).unwrap(), 0);
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
#[cfg_attr(
    target_os = "freebsd",
    ignore = "Fails on the CI; running locally on FreeBSD works, not sure what the problem is"
)]
fn connect_connection_refused() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
        let connect = match TcpStream::connect(&mut ctx, refused_address()) {
            Ok(connect) => connect,
            Err(err) => {
                assert_eq!(
                    err.kind(),
                    io::ErrorKind::ConnectionRefused,
                    "unexpected error: {:?}",
                    err
                );
                return Ok(());
            }
        };
        match connect.await {
            Ok(..) => panic!("unexpected success"),
            Err(err) => assert_eq!(
                err.kind(),
                io::ErrorKind::ConnectionRefused,
                "unexpected error: {:?}",
                err
            ),
        }
        Ok(())
    }

    let actor = actor as fn(_) -> _;
    let actor_ref = try_spawn_local(PanicSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn try_recv() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf = Vec::with_capacity(128);
        match stream.try_recv(&mut buf) {
            Ok(n) => panic!("unexpected bytes: {:?} ({})", buf, n),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => return Err(err),
        }

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

        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    stream.write_all(&DATA).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    stream.write_all(&DATA).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    stream.write_all(&DATA).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    stream.write_all(&DATA).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    stream.write_all(&DATA).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    for _ in 0..3 {
        stream.write_all(&DATA).unwrap();
    }
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn send() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let n = stream.send(&DATA).await?;
        assert_eq!(n, DATA.len());

        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    let mut buf = [0; DATA.len() + 1];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, DATA.len());
    assert_eq!(&buf[..n], DATA);
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn send_all() {
    // A lot of data to get at least two write calls.
    const DATA: &[u8] = &[213; 40 * 1024];
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
        stream.send_all(DATA).await
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();

    let mut buf = [0; 8 * 1024];
    let mut total = 0;
    loop {
        let n = stream.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        assert_eq!(&buf[..n], &DATA[total..total + n]);
        total += n;
    }
    assert_eq!(total, DATA.len());

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn send_vectored() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let bufs = &mut [
            IoSlice::new(DATA),
            IoSlice::new(DATA),
            IoSlice::new(DATA),
            IoSlice::new(DATA),
        ];
        let n = stream.send_vectored(bufs).await?;
        assert_eq!(n, 4 * DATA.len());

        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    let mut buf = [0; (4 * DATA.len()) + 1];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 4 * DATA.len());
    for n in 0..4 {
        assert_eq!(&buf[n * DATA.len()..(n + 1) * DATA.len()], DATA);
    }
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn send_vectored_all() {
    // A lot of data to get at least two write calls.
    const DATA1: &[u8] = &[213; 40 * 1023];
    const DATA2: &[u8] = &[155; 30 * 1024];
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;
        let bufs = &mut [IoSlice::new(DATA1), IoSlice::new(DATA2)];
        stream.send_vectored_all(bufs).await
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();

    let mut buf = [0; 8 * 1024];
    let mut total = 0;
    loop {
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

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn recv_vectored() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf1 = Vec::with_capacity(2 * DATA.len());
        let mut buf2 = Vec::with_capacity(2 * DATA.len() + 1);
        let bufs = [&mut buf1, &mut buf2];
        let n = stream.recv_vectored(bufs).await?;
        assert_eq!(n, 4 * DATA.len());
        assert_eq!(&buf1[..DATA.len()], DATA);
        assert_eq!(&buf1[DATA.len()..], DATA);
        assert_eq!(&buf2[..DATA.len()], DATA);
        assert_eq!(&buf2[DATA.len()..], DATA);

        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    let bufs = &mut [
        IoSlice::new(DATA),
        IoSlice::new(DATA),
        IoSlice::new(DATA),
        IoSlice::new(DATA),
    ];
    stream.write_all_vectored(bufs).unwrap();
    let mut buf = [0; 2];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    let bufs = &mut [IoSlice::new(DATA), IoSlice::new(DATA)];
    stream.write_all_vectored(bufs).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    let bufs = &mut [IoSlice::new(DATA), IoSlice::new(DATA)];
    stream.write_all_vectored(bufs).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    stream.write_all(DATA).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    for _ in 0..3 {
        stream.write_all(&DATA).unwrap();
    }
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    let (mut stream, _) = listener.accept().unwrap();
    stream.write_all(&DATA).unwrap();
    drop(stream);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
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
    let args = (address, TEST_FILE0);
    let actor_ref1 =
        try_spawn_local(PanicSupervisor, actor, args, ActorOptions::default()).unwrap();
    let (mut stream0, _) = listener.accept().unwrap();
    stream0.set_nonblocking(true).unwrap();

    let args = (address, TEST_FILE1);
    let actor_ref2 =
        try_spawn_local(PanicSupervisor, actor, args, ActorOptions::default()).unwrap();
    let (mut stream1, _) = listener.accept().unwrap();
    stream1.set_nonblocking(true).unwrap();

    let mut expected0_offset = 0;
    let expected1 = &EXPECTED1[..LENGTH];
    let mut expected1_offset = 0;

    let mut buf = vec![0; LENGTH + 1];
    for _ in 0..20 {
        // NOTE: can't use `&&` as that short circuits.
        let done0 =
            send_file_check_actor(&mut stream0, &EXPECTED0, &mut expected0_offset, &mut buf);
        let done1 =
            send_file_check_actor(&mut stream1, &expected1, &mut expected1_offset, &mut buf);

        if done0 && done1 {
            break;
        }

        sleep(Duration::from_millis(10));
    }

    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
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
    let args = (address, TEST_FILE0);
    let actor_ref1 =
        try_spawn_local(PanicSupervisor, actor, args, ActorOptions::default()).unwrap();
    let (mut stream0, _) = listener.accept().unwrap();
    stream0.set_nonblocking(true).unwrap();

    let args = (address, TEST_FILE1);
    let actor_ref2 =
        try_spawn_local(PanicSupervisor, actor, args, ActorOptions::default()).unwrap();
    let (mut stream1, _) = listener.accept().unwrap();
    stream1.set_nonblocking(true).unwrap();

    let expected0 = &EXPECTED0;
    let mut expected0_offset = OFFSET;
    let expected1 = &EXPECTED1[..OFFSET + LENGTH];
    let mut expected1_offset = OFFSET;

    let mut buf = vec![0; LENGTH + 1];
    for _ in 0..20 {
        // NOTE: can't use `&&` as that short circuits.
        let done0 =
            send_file_check_actor(&mut stream0, &expected0, &mut expected0_offset, &mut buf);
        let done1 =
            send_file_check_actor(&mut stream1, &expected1, &mut expected1_offset, &mut buf);

        if done0 && done1 {
            break;
        }

        sleep(Duration::from_millis(10));
    }

    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
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
    let args = (address, TEST_FILE0);
    let actor_ref1 =
        try_spawn_local(PanicSupervisor, actor, args, ActorOptions::default()).unwrap();
    let (mut stream0, _) = listener.accept().unwrap();
    stream0.set_nonblocking(true).unwrap();

    let args = (address, TEST_FILE1);
    let actor_ref2 =
        try_spawn_local(PanicSupervisor, actor, args, ActorOptions::default()).unwrap();
    let (mut stream1, _) = listener.accept().unwrap();
    stream1.set_nonblocking(true).unwrap();

    let mut expected0_offset = 0;
    let mut expected1_offset = 0;

    let mut buf = vec![0; 4096];
    for _ in 0..20 {
        // NOTE: can't use `&&` as that short circuits.
        let done0 =
            send_file_check_actor(&mut stream0, &EXPECTED0, &mut expected0_offset, &mut buf);
        let done1 =
            send_file_check_actor(&mut stream1, &EXPECTED1, &mut expected1_offset, &mut buf);

        if done0 && done1 {
            break;
        }

        sleep(Duration::from_millis(10));
    }

    join_many(&[actor_ref1, actor_ref2], Duration::from_secs(1)).unwrap();
}

/// Returns `true` if `actor` send all `expected` bytes to `stream`.
#[track_caller]
fn send_file_check_actor(
    stream: &mut net::TcpStream,
    expected: &[u8],
    offset: &mut usize,
    buf: &mut Vec<u8>,
) -> bool {
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

    *offset == expected.len()
}

#[test]
fn peek_vectored() {
    async fn actor(mut ctx: actor::Context<!, ThreadLocal>, address: SocketAddr) -> io::Result<()> {
        let mut stream = TcpStream::connect(&mut ctx, address)?.await?;

        let mut buf1 = Vec::with_capacity(2 * DATA.len());
        let mut buf2 = Vec::with_capacity(2 * DATA.len() + 1);
        let bufs = [&mut buf1, &mut buf2];
        let n = stream.peek_vectored(bufs).await?;
        assert_eq!(n, 4 * DATA.len());
        assert_eq!(&buf1[..DATA.len()], DATA);
        assert_eq!(&buf1[DATA.len()..], DATA);
        assert_eq!(&buf2[..DATA.len()], DATA);
        assert_eq!(&buf2[DATA.len()..], DATA);

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

        Ok(())
    }

    let listener = net::TcpListener::bind(any_local_address()).unwrap();
    let address = listener.local_addr().unwrap();

    let actor = actor as fn(_, _) -> _;
    let actor_ref =
        try_spawn_local(PanicSupervisor, actor, address, ActorOptions::default()).unwrap();

    // Once we accept the connecting the actor should be able to proceed.
    let (mut stream, _) = listener.accept().unwrap();
    let bufs = &mut [
        IoSlice::new(DATA),
        IoSlice::new(DATA),
        IoSlice::new(DATA),
        IoSlice::new(DATA),
    ];
    stream.write_all_vectored(bufs).unwrap();

    let mut buf = [0; 2];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);

    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn shutdown_read() {
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
    let stream_ref =
        try_spawn_local(NoSupervisor, stream_actor, (), ActorOptions::default()).unwrap();

    let listener_actor = listener_actor as fn(_, _) -> _;
    let s_ref = stream_ref.clone();
    let listener_ref =
        try_spawn_local(NoSupervisor, listener_actor, s_ref, ActorOptions::default()).unwrap();

    join_many(&[stream_ref, listener_ref], Duration::from_secs(1)).unwrap();
}

#[test]
fn shutdown_write() {
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
    let stream_ref =
        try_spawn_local(NoSupervisor, stream_actor, (), ActorOptions::default()).unwrap();

    let listener_actor = listener_actor as fn(_, _) -> _;
    let s_ref = stream_ref.clone();
    let listener_ref =
        try_spawn_local(NoSupervisor, listener_actor, s_ref, ActorOptions::default()).unwrap();

    join_many(&[stream_ref, listener_ref], Duration::from_secs(1)).unwrap();
}

#[test]
fn shutdown_both() {
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
    type Message = RpcMessage<TcpStream, ()>;

    async fn actor1<RT>(mut ctx: actor::Context<!, RT>, actor_ref: ActorRef<Message>)
    where
        RT: rt::Access,
    {
        let mut listener = TcpListener::bind(&mut ctx, any_local_address()).unwrap();
        let peer_address = listener.local_addr().unwrap();

        let stream = TcpStream::connect(&mut ctx, peer_address)
            .unwrap()
            .await
            .unwrap();
        let _ = actor_ref.rpc(stream).await.unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = stream.bind_to(&mut ctx).unwrap();
        stream.send_all(DATA).await.unwrap();
    }

    async fn actor2<RT>(mut ctx: actor::Context<Message, RT>)
    where
        RT: rt::Access,
    {
        let msg = ctx.receive_next().await.unwrap();
        let mut stream = msg.request;
        stream.bind_to(&mut ctx).unwrap();
        msg.response.respond(()).unwrap();
        let mut buf = Vec::with_capacity(DATA.len() + 1);
        let n = stream.recv(&mut buf).await.unwrap();
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
