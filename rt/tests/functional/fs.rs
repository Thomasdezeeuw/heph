//! Tests for the filesystem operations.

use std::io;
use std::path::PathBuf;

use heph::actor::{self, actor_fn};
use heph_rt::access::ThreadLocal;
use heph_rt::fs::{self, Advice, AllocateMode, File};
use heph_rt::io::{Read, Write};
use heph_rt::test::block_on_local_actor;

use crate::util::{temp_dir_root, temp_file};

const DATA1: &[u8] = b"Hello, World";
const DATA2: &[u8] = b"Hello, Mars!";

#[test]
fn file_read_write() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = temp_file("file_read_write");
        let file = File::create(ctx.runtime_ref(), path).await.unwrap();
        let buf = (&file).read(Vec::with_capacity(128)).await.unwrap();
        assert!(buf.is_empty());

        (&file).write_all(DATA1).await.unwrap();
        let mut buf = file.read_at(buf, 0).await.unwrap();
        assert_eq!(buf, DATA1);

        (&file).write_all_at(&DATA2[7..], 7).await.unwrap();
        buf.clear();
        let buf = file.read_at(buf, 0).await.unwrap();
        assert_eq!(buf, DATA2);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn file_from_std() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = temp_file("file_from_std");
        let file = std::fs::File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .truncate(true)
            .open(path)
            .unwrap();
        let mut file = File::from_std(ctx.runtime_ref(), file);

        file.write_all(DATA1).await.unwrap();
        let buf = file.read_at(Vec::with_capacity(128), 0).await.unwrap();
        assert_eq!(buf, DATA1);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn file_sync_all() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = temp_file("file_sync_all");
        let file = File::create(ctx.runtime_ref(), path).await.unwrap();

        (&file).write_all(DATA1).await.unwrap();
        file.sync_all().await.unwrap();
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn file_sync_data() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = temp_file("file_sync_data");
        let file = File::create(ctx.runtime_ref(), path).await.unwrap();

        (&file).write_all(DATA1).await.unwrap();
        file.sync_all().await.unwrap();
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn file_metadata() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = PathBuf::from("src/lib.rs");
        let file = File::open(ctx.runtime_ref(), path).await.unwrap();

        let metadata = file.metadata().await.unwrap();
        assert!(!metadata.is_dir());
        assert!(metadata.is_file());
        assert!(!metadata.is_symlink());
        assert!(metadata.len() >= 20_000);
        assert!(metadata.block_size() >= 512);

        let file_type = metadata.file_type();
        assert!(!file_type.is_dir());
        assert!(file_type.is_file());
        assert!(!file_type.is_symlink());
        assert!(!file_type.is_socket());
        assert!(!file_type.is_block_device());
        assert!(!file_type.is_character_device());
        assert!(!file_type.is_named_pipe());

        let permissions = metadata.permissions();
        assert!(permissions.owner_can_read());
        assert!(permissions.owner_can_write());
        assert!(!permissions.owner_can_execute());
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn file_advise() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = PathBuf::from("src/lib.rs");
        let file = File::open(ctx.runtime_ref(), path).await.unwrap();
        file.advise(0, 0, Advice::Sequential).await.unwrap(); // Entire file.

        let len = file.metadata().await.unwrap().len() as usize;
        let buf = (&file).read_n(Vec::with_capacity(len), len).await.unwrap();
        assert_eq!(buf.len(), len);

        file.advise(0, 0, Advice::DontNeed).await.unwrap(); // Entire file.
        drop(file);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn file_allocate() {
    const SIZE: usize = 1024;

    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = temp_file("file_allocate");
        let file = File::create(ctx.runtime_ref(), path).await.unwrap();

        file.allocate(0, SIZE as u32, AllocateMode::InitRangeKeepSize)
            .await
            .unwrap();
        assert_eq!(file.metadata().await.unwrap().len(), 0);

        file.allocate(0, SIZE as u32, AllocateMode::InitRange)
            .await
            .unwrap();
        assert_eq!(file.metadata().await.unwrap().len(), SIZE as u64);

        let buf = (&file)
            .read_n(Vec::with_capacity(SIZE), SIZE)
            .await
            .unwrap();
        assert_eq!(buf.len(), SIZE);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn create_dir() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let mut dir = temp_dir_root();
        dir.push("create_dir");
        fs::create_dir(ctx.runtime_ref(), dir.clone())
            .await
            .unwrap();

        dir.push("test.txt");
        let file = File::create(ctx.runtime_ref(), dir).await.unwrap();
        (&file).write(DATA1).await.unwrap();
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn rename() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let to = temp_file("rename.1");
        let from = temp_file("rename.2");
        let file = File::create(ctx.runtime_ref(), from.clone()).await.unwrap();
        (&file).write(DATA1).await.unwrap();
        drop(file);

        fs::rename(ctx.runtime_ref(), from, to.clone())
            .await
            .unwrap();

        let file = File::open(ctx.runtime_ref(), to).await.unwrap();
        let buf = (&file)
            .read(Vec::with_capacity(DATA1.len() + 1))
            .await
            .unwrap();
        assert_eq!(buf, DATA1);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn remove_file() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = temp_file("remove_file");
        let file = File::create(ctx.runtime_ref(), path.clone()).await.unwrap();
        (&file).write(DATA1).await.unwrap();
        drop(file);

        fs::remove_file(ctx.runtime_ref(), path.clone())
            .await
            .unwrap();

        let err = File::open(ctx.runtime_ref(), path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}

#[test]
fn remove_dir() {
    async fn actor(ctx: actor::Context<!, ThreadLocal>) {
        let path = temp_file("remove_dir");
        std::fs::create_dir(&path).unwrap();

        fs::remove_dir(ctx.runtime_ref(), path.clone())
            .await
            .unwrap();

        let err = std::fs::read_dir(path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    block_on_local_actor(actor_fn(actor), ()).unwrap();
}
