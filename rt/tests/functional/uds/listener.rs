//! Tests for `UnixListener`.

use std::io;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph_rt::net::uds::{UnixAddr, UnixListener, UnixStream};
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{block_on_local_actor, join, try_spawn_local, PanicSupervisor};
use heph_rt::util::next;
use heph_rt::ThreadLocal;
use heph_rt::{self as rt};

use crate::util::temp_file;

const DATA: &[u8] = b"Hello world";

#[test]
fn accept() {
    async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let path = temp_file("uds_listener_accept");
        let address = UnixAddr::from_pathname(path)?;
        let listener = UnixListener::bind(ctx.runtime_ref(), address.clone()).await?;

        let stream = UnixStream::connect(ctx.runtime_ref(), address).await?;

        let (client, _) = listener.accept().await?;

        let (_, n) = stream.send(DATA).await?;
        assert_eq!(n, DATA.len());
        let buf = client.recv(Vec::with_capacity(DATA.len() + 1)).await?;
        assert_eq!(buf.len(), DATA.len());
        assert_eq!(buf, DATA);

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
fn incoming() {
    async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let path = temp_file("uds_listener_incoming");
        let address = UnixAddr::from_pathname(path)?;
        let listener = UnixListener::bind(ctx.runtime_ref(), address.clone()).await?;

        let mut incoming = listener.incoming();

        let stream = UnixStream::connect(ctx.runtime_ref(), address).await?;

        let client = next(&mut incoming).await.unwrap()?;

        let (_, n) = stream.send(DATA).await?;
        assert_eq!(n, DATA.len());
        let buf = client.recv(Vec::with_capacity(DATA.len() + 1)).await?;
        assert_eq!(buf.len(), DATA.len());
        assert_eq!(buf, DATA);

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
fn listener_from_std() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) -> io::Result<()> {
        let path = temp_file("uds.listener_from_std");

        let listener = std::os::unix::net::UnixListener::bind(path)?;
        let listener = UnixListener::from_std(ctx.runtime_ref(), listener);

        let address = listener.local_addr()?;
        let stream = UnixStream::connect(ctx.runtime_ref(), address).await?;

        let (client, _) = listener.accept().await?;

        let (_, n) = stream.send(DATA).await?;
        assert_eq!(n, DATA.len());
        let buf = client.recv(Vec::with_capacity(DATA.len() + 1)).await?;
        assert_eq!(buf.len(), DATA.len());
        assert_eq!(buf, DATA);

        Ok(())
    }

    block_on_local_actor(actor_fn(actor), ());
}
