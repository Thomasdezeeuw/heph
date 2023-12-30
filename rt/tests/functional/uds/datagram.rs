//! Tests for `UnixDatagram`.

use std::io;
use std::net::Shutdown;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph_rt::net::uds::{UnixAddr, UnixDatagram};
use heph_rt::net::Unconnected;
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
        let (s1, s2) = UnixDatagram::pair(ctx.runtime_ref())?;

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
fn bound() {
    async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let path1 = temp_file("uds.bound1");
        let path2 = temp_file("uds.bound2");
        let address1 = UnixAddr::from_pathname(path1)?;
        let address2 = UnixAddr::from_pathname(path2)?;
        let listener = UnixDatagram::bind(ctx.runtime_ref(), address1.clone()).await?;

        // Addresses must point to each other.
        assert_eq!(listener.local_addr()?, address1);
        assert!(listener.peer_addr().is_err());

        let socket = UnixDatagram::bind(ctx.runtime_ref(), address2.clone()).await?;
        let socket = socket.connect(address1.clone()).await?;
        assert_eq!(socket.local_addr()?, address2);
        assert_eq!(socket.peer_addr()?, address1);

        let (_, n) = listener.send_to(DATA, address2).await?;
        assert_eq!(n, DATA.len());
        let buf = socket.recv(Vec::with_capacity(DATA.len() + 1)).await?;
        assert_eq!(buf.len(), DATA.len());
        assert_eq!(buf, DATA);

        // No errors.
        assert!(listener.take_error()?.is_none());
        assert!(socket.take_error()?.is_none());

        Ok(())
    }

    let actor = actor_fn(actor);
    let actor_ref = try_spawn_local(PanicSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn socket_from_std() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
        let path1 = temp_file("uds.socket_from_std1");
        let path2 = temp_file("uds.socket_from_std2");
        let socket = std::os::unix::net::UnixDatagram::bind(path1)?;
        let socket = UnixDatagram::<Unconnected>::from_std(ctx.runtime_ref(), socket);
        let local_address = socket.local_addr()?;

        let peer = std::os::unix::net::UnixDatagram::bind(path2)?;
        let peer_address = UnixAddr::from_pathname(peer.local_addr()?.as_pathname().unwrap())?;

        let (_, bytes_written) = socket.send_to(DATA, peer_address).await?;
        assert_eq!(bytes_written, DATA.len());

        let mut buf = vec![0; DATA.len() + 2];
        let (n, address) = peer.recv_from(&mut buf)?;
        assert_eq!(n, DATA.len());
        assert_eq!(&buf[..n], DATA);
        assert_eq!(address.as_pathname(), local_address.as_pathname());

        Ok(())
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}
