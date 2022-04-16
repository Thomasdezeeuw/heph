//! Tests for the Unix pipe.

use std::io::{self, IoSlice};
use std::time::Duration;

use heph::spawn::ActorOptions;
use heph::{actor, ActorRef};
use heph_rt::pipe::{self, Receiver, Sender};
use heph_rt::test::{join, join_many, try_spawn_local, PanicSupervisor};
use heph_rt::{self as rt, Bound};

const DATA: &[u8] = b"Hello world";
const DATAV: &[&[u8]] = &[b"Hello world!", b" ", b"From mars."];
const DATAV_LEN: usize = DATAV[0].len() + DATAV[1].len() + DATAV[2].len();

#[test]
fn smoke() {
    async fn actor<RT>(mut ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (mut sender, mut receiver) = pipe::new(&mut ctx)?;

        let n = sender.write(DATA).await?;
        assert_eq!(n, DATA.len());
        drop(sender);

        let mut buf = Vec::with_capacity(DATA.len() + 1);
        let n = receiver.read(&mut buf).await?;
        assert_eq!(n, DATA.len());
        assert_eq!(buf, DATA);
        Ok(())
    }

    #[allow(trivial_casts)]
    let actor = actor as fn(_) -> _;
    let actor_ref = try_spawn_local(PanicSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn write_all_read_n() {
    // Linux usually buffers 16 pages (4kb) for a
    const DATA: &[u8] = &[213; 17 * 4096];

    async fn writer<RT>(
        mut ctx: actor::Context<Receiver, RT>,
        reader: ActorRef<Receiver>,
    ) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (mut sender, receiver) = pipe::new(&mut ctx)?;

        reader.send(receiver).await.unwrap();

        sender.write_all(DATA).await?;
        drop(sender);
        Ok(())
    }

    async fn reader<RT>(mut ctx: actor::Context<Receiver, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let mut receiver = ctx.receive_next().await.unwrap();
        receiver.bind_to(&mut ctx)?;

        let mut buf = Vec::with_capacity(DATA.len() + 1);
        receiver.read_n(&mut buf, DATA.len()).await?;
        Ok(())
    }

    #[allow(trivial_casts)]
    let reader = reader as fn(_) -> _;
    let reader_ref = try_spawn_local(PanicSupervisor, reader, (), ActorOptions::default()).unwrap();

    #[allow(trivial_casts)]
    let writer = writer as fn(_, _) -> _;
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
        mut ctx: actor::Context<Receiver, RT>,
        reader: ActorRef<Receiver>,
    ) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (mut sender, receiver) = pipe::new(&mut ctx)?;

        reader.send(receiver).await.unwrap();

        let bufs = &mut [
            IoSlice::new(&DATA[0]),
            IoSlice::new(&DATA[1]),
            IoSlice::new(&DATA[2]),
        ];
        sender.write_vectored_all(bufs).await?;
        drop(sender);
        Ok(())
    }

    async fn reader<RT>(mut ctx: actor::Context<Receiver, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let mut receiver = ctx.receive_next().await.unwrap();
        receiver.bind_to(&mut ctx)?;

        let mut bufs = &mut [
            Vec::with_capacity(8 * 4096),
            Vec::with_capacity(6 * 4096),
            Vec::with_capacity((4 * 4096) + 1),
        ];
        receiver.read_n_vectored(&mut bufs, DATA_LEN).await?;
        Ok(())
    }

    #[allow(trivial_casts)]
    let reader = reader as fn(_) -> _;
    let reader_ref = try_spawn_local(PanicSupervisor, reader, (), ActorOptions::default()).unwrap();

    #[allow(trivial_casts)]
    let writer = writer as fn(_, _) -> _;
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
#[ignore]
fn vectored_io() {
    async fn actor<RT>(mut ctx: actor::Context<!, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (mut sender, mut receiver) = pipe::new(&mut ctx)?;

        let bufs = &mut [
            IoSlice::new(DATAV[0]),
            IoSlice::new(DATAV[1]),
            IoSlice::new(DATAV[2]),
        ];
        let n = sender.write_vectored(bufs).await?;
        assert_eq!(n, DATA.len());
        drop(sender);

        let mut buf1 = Vec::with_capacity(DATAV[0].len());
        let mut buf2 = Vec::with_capacity(DATAV[1].len());
        let mut buf3 = Vec::with_capacity(DATAV[2].len() + 2);
        let mut bufs = [&mut buf1, &mut buf2, &mut buf3];
        let n = receiver.read_vectored(&mut bufs).await?;
        assert_eq!(n, DATAV_LEN);
        assert_eq!(buf1, DATAV[0]);
        assert_eq!(buf2, DATAV[1]);
        assert_eq!(buf3, DATAV[2]);
        Ok(())
    }

    #[allow(trivial_casts)]
    let actor = actor as fn(_) -> _;
    let actor_ref = try_spawn_local(PanicSupervisor, actor, (), ActorOptions::default()).unwrap();
    join(&actor_ref, Duration::from_secs(1)).unwrap();
}

#[test]
fn actor_bound() {
    async fn creator<RT>(
        mut ctx: actor::Context<!, RT>,
        write_ref: ActorRef<Sender>,
        reader_ref: ActorRef<Receiver>,
    ) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let (sender, receiver) = pipe::new(&mut ctx)?;
        reader_ref.send(receiver).await.unwrap();
        write_ref.send(sender).await.unwrap();
        Ok(())
    }

    async fn writer<RT>(mut ctx: actor::Context<Sender, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let mut sender = ctx.receive_next().await.unwrap();
        sender.bind_to(&mut ctx)?;

        sender.write_all(DATA).await
    }

    async fn reader<RT>(mut ctx: actor::Context<Receiver, RT>) -> io::Result<()>
    where
        RT: rt::Access,
    {
        let mut receiver = ctx.receive_next().await.unwrap();
        receiver.bind_to(&mut ctx)?;

        let mut buf = Vec::with_capacity(DATA.len() + 1);
        receiver.read_n(&mut buf, DATA.len()).await?;
        assert_eq!(buf, DATA);
        Ok(())
    }

    #[allow(trivial_casts)]
    let reader = reader as fn(_) -> _;
    let reader_ref = try_spawn_local(PanicSupervisor, reader, (), ActorOptions::default()).unwrap();

    #[allow(trivial_casts)]
    let writer = writer as fn(_) -> _;
    let writer_ref = try_spawn_local(PanicSupervisor, writer, (), ActorOptions::default()).unwrap();

    #[allow(trivial_casts)]
    let creator = creator as fn(_, _, _) -> _;
    let creator_ref = try_spawn_local(
        PanicSupervisor,
        creator,
        (writer_ref.clone(), reader_ref.clone()),
        ActorOptions::default(),
    )
    .unwrap();

    // Can't use `join_many` due to the differening message types.
    join(&creator_ref, Duration::from_secs(1)).unwrap();
    join(&writer_ref, Duration::from_secs(1)).unwrap();
    join(&reader_ref, Duration::from_secs(1)).unwrap();
}
