//! Tests for `UnixStream`.

use std::io::{self, Read};
use std::net::Shutdown;
use std::os::unix::net;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph_rt::net::uds::{UnixAddr, UnixStream};
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{block_on_local_actor, join, try_spawn_local, PanicSupervisor};
use heph_rt::ThreadLocal;
use heph_rt::{self as rt};

use crate::util::temp_file;

const DATA: &[u8] = b"Hello world";
const DATA2: &[u8] = b"Hello mars";

#[test]
fn pair() {
    async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (s1, s2) = UnixStream::pair(ctx.runtime_ref())?;

        // Addresses must point to each other.
        let s1_local = s1.local_addr()?;
        let s1_peer = s1.peer_addr()?;
        let s2_local = s2.local_addr()?;
        let s2_peer = s2.peer_addr()?;
        assert_eq!(s1_local, s2_peer);
        assert_eq!(s1_peer, s2_local);

        // Send to one arrives at the other.
        let (_, n) = s1.send(DATA).await?;
        assert_eq!(n, DATA.len());
        let mut buf = s2.recv(Vec::with_capacity(DATA.len() + 1)).await?;
        assert_eq!(buf.len(), DATA.len());
        assert_eq!(buf, DATA);
        buf.clear();

        // Same as above, but then in the other direction.
        let (_, n) = s2.send(DATA2).await?;
        assert_eq!(n, DATA2.len());
        let mut buf = s1.recv(buf).await?;
        assert_eq!(buf.len(), DATA2.len());
        assert_eq!(buf, DATA2);
        buf.clear();

        // Shutdown.
        s1.shutdown(Shutdown::Both).await?;
        s2.shutdown(Shutdown::Both).await?;

        // No errors.
        assert!(s1.take_error()?.is_none());
        assert!(s2.take_error()?.is_none());

        Ok(())
    }

    let actor = actor_fn(actor);
    let actor_ref = try_spawn_local(PanicSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn connect() {
    async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let path = temp_file("uds_stream_bound");
        let listener = net::UnixListener::bind(&path)?;

        let address = UnixAddr::from_pathname(path)?;
        let stream = UnixStream::connect(ctx.runtime_ref(), address).await?;

        let (mut client, _) = listener.accept()?;

        let (_, n) = stream.send(DATA).await?;
        assert_eq!(n, DATA.len());
        let mut buf = vec![0; DATA.len() + 1];
        let n = client.read(&mut buf)?;
        assert_eq!(n, DATA.len());
        assert_eq!(&buf[..n], DATA);

        // No errors.
        assert!(listener.take_error()?.is_none());
        assert!(client.take_error()?.is_none());
        assert!(stream.take_error()?.is_none());

        Ok(())
    }

    let actor = actor_fn(actor);
    let actor_ref = try_spawn_local(PanicSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn stream_from_std() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
        let (mut peer, stream) = net::UnixStream::pair()?;
        let stream = UnixStream::from_std(ctx.runtime_ref(), stream);

        stream.send_all(DATA).await?;

        let mut buf = vec![0; DATA.len() + 2];
        let n = peer.read(&mut buf)?;
        assert_eq!(n, DATA.len());
        assert_eq!(&buf[..n], DATA);

        Ok(())
    }

    block_on_local_actor(actor_fn(actor), ());
}
