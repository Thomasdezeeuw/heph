//! Tests for the Unix pipe.

use std::io;
use std::time::Duration;

use heph::actor::{self, actor_fn};
use heph::ActorRef;
use heph_rt::io::{Read, Write};
use heph_rt::pipe::{self, Receiver};
use heph_rt::spawn::ActorOptions;
use heph_rt::test::{join, join_many, try_spawn_local, PanicSupervisor};
use heph_rt::{self as rt};

const DATA: &[u8] = b"Hello world";
const DATAV: &[&[u8]] = &[b"Hello world!", b" ", b"From mars."];
const DATAV_LEN: usize = DATAV[0].len() + DATAV[1].len() + DATAV[2].len();

#[test]
fn smoke() {
    async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (sender, receiver) = pipe::new(ctx.runtime_ref())?;

        let (_, n) = (&sender).write(DATA).await?;
        assert_eq!(n, DATA.len());
        drop(sender);

        let buf = (&receiver).read(Vec::with_capacity(DATA.len() + 1)).await?;
        assert_eq!(buf.len(), DATA.len());
        assert_eq!(buf, DATA);
        Ok(())
    }

    let actor = actor_fn(actor);
    let actor_ref = try_spawn_local(PanicSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn write_all_read_n() {
    // Linux usually buffers 16 pages (4kb) for a
    const DATA: &[u8] = &[213; 17 * 4096];

    async fn writer<RT>(
        ctx: actor::Context<Receiver, RT>,
        reader: ActorRef<Receiver>,
    ) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (sender, receiver) = pipe::new(ctx.runtime_ref())?;

        reader.send(receiver).await.unwrap();

        (&sender).write_all(DATA).await?;
        drop(sender);
        Ok(())
    }

    async fn reader<RT>(mut ctx: actor::Context<Receiver, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let receiver = ctx.receive_next().await.unwrap();

        let buf = (&receiver)
            .read_n(Vec::with_capacity(DATA.len() + 1), DATA.len())
            .await?;
        assert_eq!(buf, DATA);
        Ok(())
    }

    let reader = actor_fn(reader);
    let reader_ref = try_spawn_local(PanicSupervisor, reader, (), ActorOptions::default()).unwrap();

    let writer = actor_fn(writer);
    let writer_ref = try_spawn_local(
        PanicSupervisor,
        writer,
        reader_ref.clone(),
        ActorOptions::default(),
    )
    .unwrap();
    join_many(&[reader_ref, writer_ref], Duration::from_secs(1)).unwrap();
}

#[test]
fn write_vectored_all_read_n_vectored() {
    // Linux usually buffers 16 pages (4kb) for a
    const DATA: &[&[u8]] = &[&[213; 8 * 4096], &[123; 6 * 4096], &[86; 4 * 4096]];
    const DATA_LEN: usize = DATA[0].len() + DATA[1].len() + DATA[2].len();

    async fn writer<RT>(
        ctx: actor::Context<Receiver, RT>,
        reader: ActorRef<Receiver>,
    ) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (sender, receiver) = pipe::new(ctx.runtime_ref())?;

        reader.send(receiver).await.unwrap();

        let bufs = [DATA[0], DATA[1], DATA[2]];
        (&sender).write_vectored_all(bufs).await?;
        drop(sender);
        Ok(())
    }

    async fn reader<RT>(mut ctx: actor::Context<Receiver, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let receiver = ctx.receive_next().await.unwrap();

        let bufs = [
            Vec::with_capacity(8 * 4096),
            Vec::with_capacity(6 * 4096),
            Vec::with_capacity((4 * 4096) + 1),
        ];
        let [buf1, buf2, buf3] = (&receiver).read_n_vectored(bufs, DATA_LEN).await?;
        debug_assert!(buf1 == DATA[0]);
        debug_assert!(buf2 == DATA[1]);
        debug_assert!(buf3 == DATA[2]);
        Ok(())
    }

    let reader = actor_fn(reader);
    let reader_ref = try_spawn_local(PanicSupervisor, reader, (), ActorOptions::default()).unwrap();

    let writer = actor_fn(writer);
    let writer_ref = try_spawn_local(
        PanicSupervisor,
        writer,
        reader_ref.clone(),
        ActorOptions::default(),
    )
    .unwrap();

    join_many(&[reader_ref, writer_ref], Duration::from_secs(1)).unwrap();
}

#[test]
fn vectored_io() {
    async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (sender, receiver) = pipe::new(ctx.runtime_ref())?;

        let bufs = [DATAV[0], DATAV[1], DATAV[2]];
        let (_, n) = (&sender).write_vectored(bufs).await?;
        assert_eq!(n, DATAV_LEN);
        drop(sender);

        let bufs = [
            Vec::with_capacity(DATAV[0].len()),
            Vec::with_capacity(DATAV[1].len()),
            Vec::with_capacity(DATAV[2].len() + 2),
        ];
        let [buf1, buf2, buf3] = (&receiver).read_vectored(bufs).await?;
        assert!(buf1 == DATAV[0]);
        assert!(buf2 == DATAV[1]);
        assert!(buf3 == DATAV[2]);
        Ok(())
    }

    let actor = actor_fn(actor);
    let actor_ref = try_spawn_local(PanicSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}
