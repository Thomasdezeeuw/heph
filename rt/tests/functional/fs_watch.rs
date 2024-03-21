//! Tests for the watching the filesystem.

use std::path::{Path, PathBuf};

use heph::actor::{self, actor_fn};
use heph_rt::access::ThreadLocal;
use heph_rt::fs::watch::{Event, Interest, Recursive, Watch};
use heph_rt::test::block_on_local_actor;

use crate::util::temp_dir;

const FILE_NAME: &str = "test.txt";
const DIR_NAME: &str = "some_dir";

#[test]
fn watched_directory_file_created() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_created",
            |dir| (dir.clone(), Interest::CREATE, Some(Recursive::No), dir),
            |dir| {
                let path = dir.join(FILE_NAME);
                std::fs::write(&path, b"Hello, World!").expect("failed to create file");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    file_created: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_dir_created() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_created",
            |dir| (dir.clone(), Interest::CREATE, Some(Recursive::No), dir),
            |dir| {
                let path = dir.join(DIR_NAME);
                std::fs::create_dir(&path).expect("failed to create directory");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: DIR_NAME,
                    is_dir: true,
                    file_created: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_file_opened() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_opened",
            |dir| {
                let path = dir.join(FILE_NAME);
                std::fs::write(&path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::OPEN, Some(Recursive::No), path)
            },
            |path| {
                let file = std::fs::File::open(&path).expect("failed to open file");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    opened: true,
                    ..Default::default()
                }]
            },
            drop, // Close file and drop path.
        ),
    );
}

#[test]
fn watched_directory_dir_opened() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_opened",
            |dir| {
                let path = dir.join(DIR_NAME);
                std::fs::create_dir(&path).expect("failed to create directory");
                (dir, Interest::OPEN, Some(Recursive::No), path)
            },
            |path| {
                let dir = std::fs::read_dir(&path).expect("failed to open directory");
                (dir, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: DIR_NAME,
                    is_dir: true,
                    opened: true,
                    ..Default::default()
                }]
            },
            drop, // Close directory and drop path.
        ),
    );
}

#[test]
fn watched_directory_file_accessed() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_accessed",
            |dir| {
                let path = dir.join(FILE_NAME);
                std::fs::write(&path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::ACCESS, Some(Recursive::No), path)
            },
            |path| {
                let mut file = std::fs::File::open(&path).expect("failed to open file");
                let mut buf = vec![0; 32];
                let _ = std::io::Read::read(&mut file, &mut buf).expect("failed to read");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    accessed: true,
                    ..Default::default()
                }]
            },
            drop, // Close file and drop path.
        ),
    );
}

#[test]
fn watched_directory_dir_accessed() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_accessed",
            |dir| {
                let path = dir.join(DIR_NAME);
                std::fs::create_dir(&path).expect("failed to create directory");
                let file_path = path.join("file.txt");
                std::fs::write(&file_path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::ACCESS, Some(Recursive::No), path)
            },
            |path| {
                let mut dir = std::fs::read_dir(&path).expect("failed to open directory");
                let _ = dir
                    .next()
                    .expect("missing file")
                    .expect("failed to read dir");
                (dir, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: DIR_NAME,
                    is_dir: true,
                    accessed: true,
                    ..Default::default()
                }]
            },
            drop, // Close directory and drop path.
        ),
    );
}

#[test]
fn watched_directory_file_modified() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_modified",
            |dir| {
                let path = dir.join(FILE_NAME);
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(&path)
                    .expect("failed to open file");
                let state = (file, path);
                (dir.clone(), Interest::MODIFY, Some(Recursive::No), state)
            },
            |(mut file, path)| {
                std::io::Write::write(&mut file, b"\nHello, again!").expect("failed to write");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    modified: true,
                    ..Default::default()
                }]
            },
            drop, // Close file and drop path.
        ),
    );
}

// NOTE: `IN_MODIFY` doesn't trigger for directory within a watched directory.

#[test]
fn watched_directory_file_metadata_changed() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_metadata_changed",
            |dir| {
                let path = dir.join(FILE_NAME);
                std::fs::write(&path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::METADATA, Some(Recursive::No), path)
            },
            |path| {
                let file = std::fs::File::open(&path).expect("failed to open file");
                let metadata = file.metadata().expect("failed to get metadata");
                let mut permissions = metadata.permissions();
                permissions.set_readonly(!permissions.readonly());
                file.set_permissions(permissions)
                    .expect("failed to set permissions");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    metadata_changed: true,
                    ..Default::default()
                }]
            },
            drop, // Close file and drop path.
        ),
    );
}

#[test]
fn watched_directory_dir_metadata_changed() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_metadata_changed",
            |dir| {
                let path = dir.join(DIR_NAME);
                std::fs::create_dir(&path).expect("failed to create directory");
                let file_path = path.join("file.txt");
                std::fs::write(&file_path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::METADATA, Some(Recursive::No), path)
            },
            |path| {
                let file = std::fs::File::open(&path).expect("failed to open file");
                let metadata = file.metadata().expect("failed to get metadata");
                let mut permissions = metadata.permissions();
                permissions.set_readonly(!permissions.readonly());
                file.set_permissions(permissions.clone())
                    .expect("failed to set permissions");
                // Reverse it outwise the directory can't be deleted.
                permissions.set_readonly(!permissions.readonly());
                file.set_permissions(permissions)
                    .expect("failed to set permissions");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: DIR_NAME,
                    is_dir: true,
                    metadata_changed: true,
                    ..Default::default()
                }]
            },
            drop, // Close directory and drop path.
        ),
    );
}

#[test]
fn watched_directory_file_moved_from() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_moved_from",
            |dir| {
                let path = dir.join(FILE_NAME);
                std::fs::write(&path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::MOVE_FROM, Some(Recursive::No), path)
            },
            |path| {
                let to = path
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("watched_directory_file_moved_from.txt");
                std::fs::rename(&path, to).expect("failed to move file");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    file_moved_from: true,
                    file_moved: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_dir_moved_from() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_moved_from",
            |dir| {
                let path = dir.join(DIR_NAME);
                std::fs::create_dir(&path).expect("failed to create directory");
                (dir, Interest::MOVE_FROM, Some(Recursive::No), path)
            },
            |path| {
                let to = path
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("watched_directory_dir_moved_from.moved");
                std::fs::rename(&path, to).expect("failed to move directory");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: DIR_NAME,
                    is_dir: true,
                    file_moved_from: true,
                    file_moved: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_file_moved_to() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_moved_to",
            |dir| {
                let from = dir
                    .parent()
                    .unwrap()
                    .join("watched_directory_file_moved_to.txt");
                std::fs::write(&from, b"Hello, World!").expect("failed to create file");
                let to = dir.join(FILE_NAME);
                (dir, Interest::MOVE_INTO, Some(Recursive::No), (from, to))
            },
            |(from, to)| {
                std::fs::rename(from, &to).expect("failed to move file");
                to
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    file_moved_into: true,
                    file_moved: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_dir_moved_to() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_moved_to",
            |dir| {
                let from = dir
                    .parent()
                    .unwrap()
                    .join("watched_directory_dir_moved_to.moved");
                std::fs::create_dir(&from).expect("failed to create directory");
                let to = dir.join(DIR_NAME);
                (dir, Interest::MOVE_INTO, Some(Recursive::No), (from, to))
            },
            |(from, to)| {
                std::fs::rename(from, &to).expect("failed to move directory");
                to
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: DIR_NAME,
                    is_dir: true,
                    file_moved_into: true,
                    file_moved: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_moved() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_moved",
            |dir| (dir.clone(), Interest::MOVE_SELF, Some(Recursive::No), dir),
            |dir| {
                let to = dir.parent().unwrap().join("watched_directory_moved_new");
                std::fs::rename(&dir, to).expect("failed to move directory");
                dir
            },
            |dir| {
                vec![ExpectEvent {
                    full_path: dir.clone(),
                    moved: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_file_closed_no_write() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_closed_no_write",
            |dir| {
                let path = dir.join(FILE_NAME);
                std::fs::write(&path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::CLOSE_NOWRITE, Some(Recursive::No), path)
            },
            |path| {
                let file = std::fs::File::open(&path).expect("failed to open file");
                drop(file);
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    closed_no_write: true,
                    closed: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_dir_closed_no_write() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_closed_no_write",
            |dir| {
                let path = dir.join(DIR_NAME);
                std::fs::create_dir(&path).expect("failed to create directory");
                (dir, Interest::CLOSE_NOWRITE, Some(Recursive::No), path)
            },
            |path| {
                let dir = std::fs::read_dir(&path).expect("failed to open directory");
                for entry in dir {
                    entry.expect("failed to read entry");
                }
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: DIR_NAME,
                    is_dir: true,
                    closed_no_write: true,
                    closed: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_file_closed_write() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_closed_write",
            |dir| {
                let path = dir.join(FILE_NAME);
                std::fs::write(&path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::CLOSE_WRITE, Some(Recursive::No), path)
            },
            |path| {
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .expect("failed to open file");
                drop(file);
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    closed_write: true,
                    closed: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

// NOTE: no close_write event can be generated for directories.

#[test]
fn watched_directory_dir_deleted() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_deleted",
            |dir| {
                let path = dir.join(DIR_NAME);
                std::fs::create_dir(&path).expect("failed to create directory");
                (dir, Interest::DELETE, Some(Recursive::No), path)
            },
            |path| {
                std::fs::remove_dir(&path).expect("failed to delete directory");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: DIR_NAME,
                    is_dir: true,
                    file_deleted: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_file_deleted() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_file_deleted",
            |dir| {
                let path = dir.join(FILE_NAME);
                std::fs::write(&path, b"Hello, World!").expect("failed to create file");
                (dir, Interest::DELETE, Some(Recursive::No), path)
            },
            |path| {
                std::fs::remove_file(&path).expect("failed to delete file");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    file_path: FILE_NAME,
                    file_deleted: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_directory_deleted() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_directory_dir_deleted",
            |dir| (dir.clone(), Interest::DELETE_SELF, Some(Recursive::No), dir),
            |path| {
                std::fs::remove_dir(&path).expect("failed to delete direcory");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    deleted: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_file_opened() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_opened",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::OPEN, None, path)
            },
            |path| {
                let file = std::fs::File::open(&path).expect("failed to open file");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    opened: true,
                    ..Default::default()
                }]
            },
            drop, // Close file and drop path.
        ),
    );
}

#[test]
fn watched_file_accessed() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_accessed",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::ACCESS, None, path)
            },
            |path| {
                let mut file = std::fs::File::open(&path).expect("failed to open file");
                let mut buf = vec![0; 32];
                let _ = std::io::Read::read(&mut file, &mut buf).expect("failed to read");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    accessed: true,
                    ..Default::default()
                }]
            },
            drop, // Close file and drop path.
        ),
    );
}

#[test]
fn watched_file_modified() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_modified",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::MODIFY, None, path)
            },
            |path| {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .expect("failed to open file");
                std::io::Write::write(&mut file, b"\nHello, again!").expect("failed to write");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    modified: true,
                    ..Default::default()
                }]
            },
            drop, // Close file and drop path.
        ),
    );
}

#[test]
fn watched_file_metadata_changed() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_metadata_change",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::METADATA, None, path)
            },
            |path| {
                let file = std::fs::File::open(&path).expect("failed to open file");
                let metadata = file.metadata().expect("failed to get metadata");
                let mut permissions = metadata.permissions();
                permissions.set_readonly(!permissions.readonly());
                file.set_permissions(permissions)
                    .expect("failed to set permissions");
                (file, path)
            },
            |(_, path)| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    metadata_changed: true,
                    ..Default::default()
                }]
            },
            drop, // Close file and drop path.
        ),
    );
}

#[test]
fn watched_file_moved() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_moved",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::MOVE_SELF, None, path)
            },
            |path| {
                let to = path.parent().unwrap().join("renamed.txt");
                std::fs::rename(&path, &to).expect("failed to rename file");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    moved: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_file_closed_nowrite() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_closed_nowrite",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::CLOSE_NOWRITE, None, path)
            },
            |path| {
                let file = std::fs::File::open(&path).expect("failed to open file");
                drop(file);
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    closed_no_write: true,
                    closed: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_file_closed_write() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_closed_write",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::CLOSE_WRITE, None, path)
            },
            |path| {
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .expect("failed to open file");
                drop(file);
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    closed_write: true,
                    closed: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_file_closed_after_read() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_closed_after_read",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::CLOSE, None, path)
            },
            |path| {
                let file = std::fs::File::open(&path).expect("failed to open file");
                drop(file);
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    closed_no_write: true,
                    closed: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_file_closed_after_write() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_closed_after_write",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::CLOSE, None, path)
            },
            |path| {
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .expect("failed to open file");
                drop(file);
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    closed_write: true,
                    closed: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_file_deleted() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_deleted",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::DELETE_SELF, None, path)
            },
            |path| {
                std::fs::remove_file(&path).expect("failed to delete file");
                path
            },
            |path| {
                vec![ExpectEvent {
                    full_path: path.clone(),
                    deleted: true,
                    ..Default::default()
                }]
            },
            drop, // Drop path.
        ),
    );
}

#[test]
fn watched_file_all() {
    block_on_local_actor(
        actor_fn(actor),
        (
            "fs_watch.watched_file_deleted",
            |tmp_dir| {
                let path = tmp_dir.join(FILE_NAME);
                std::fs::write(path.clone(), b"Hello, World!").expect("failed to create file");
                (path.clone(), Interest::ALL, None, path)
            },
            |path| {
                // Open.
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .expect("failed to open file");
                // Modified.
                std::io::Write::write(&mut file, b"\nHello, again!").expect("failed to write");
                // Metadata.
                let metadata = file.metadata().expect("failed to get metadata");
                let mut permissions = metadata.permissions();
                permissions.set_readonly(!permissions.readonly());
                file.set_permissions(permissions)
                    .expect("failed to set permissions");
                // Closed (write).
                drop(file);
                // Moved.
                let to = path.parent().unwrap().join("renamed.txt");
                std::fs::rename(&path, &to).expect("failed to rename file");
                path
            },
            |path| {
                vec![
                    ExpectEvent {
                        full_path: path.clone(),
                        opened: true,
                        ..Default::default()
                    },
                    ExpectEvent {
                        full_path: path.clone(),
                        modified: true,
                        ..Default::default()
                    },
                    ExpectEvent {
                        full_path: path.clone(),
                        metadata_changed: true,
                        ..Default::default()
                    },
                    ExpectEvent {
                        full_path: path.clone(),
                        closed_write: true,
                        closed: true,
                        ..Default::default()
                    },
                    ExpectEvent {
                        full_path: path.clone(),
                        moved: true,
                        ..Default::default()
                    },
                ]
            },
            drop, // Drop path.
        ),
    );
}

/// Test actors.
///
/// Arguments:
///  * `tmp_name`: Name for the temporary directory.
///  * `setup`: Initial setup before the watch is started.
///  * `create_event`: performed to create a watch event.
///  * `expected_events`: used to determine the expected events.
///  * `finish`: final cleanup.
async fn actor<S, S1, E, S2, EE, C>(
    ctx: actor::Context<!, ThreadLocal>,
    tmp_name: &'static str,
    setup: S,
    create_event: E,
    expected_events: EE,
    cleanup: C,
) where
    S: FnOnce(PathBuf) -> (PathBuf, Interest, Option<Recursive>, S1),
    E: FnOnce(S1) -> S2,
    EE: for<'a> FnOnce(&'a S2) -> Vec<ExpectEvent<'a>>,
    C: FnOnce(S2),
{
    // Create a temporary working directory.
    let tmp_dir = temp_dir(tmp_name);
    std::fs::create_dir_all(&tmp_dir).expect("failed to create directory");
    // Test specific setup.
    let (watch_path, interest, recursive, state) = setup(tmp_dir);

    // Create our watch.
    let mut watch = Watch::new(ctx.runtime_ref()).expect("failed to create fs::Watch");
    if let Some(recursive) = recursive {
        watch
            .watch_directory(watch_path, interest, recursive)
            .expect("failed to watch directory");
    } else {
        watch
            .watch_file(watch_path, interest)
            .expect("failed to watch file");
    }

    // Trigger the event, test specific.
    let state = create_event(state);

    // Check if we got all the events we expect.
    let expected = expected_events(&state);
    expect_events(&mut watch, &expected).await;

    // Cleanup.
    cleanup(state);
}

async fn expect_events(watch: &mut Watch, expected: &[ExpectEvent<'_>]) {
    let mut events = watch.events().await.expect("failed to read events");
    assert_eq!(events.len(), expected.len());

    for expected in expected {
        let event = events.next().expect("missing file creation event");
        assert_eq!(event, *expected);
        let full_path = events.path_for(event);
        assert_eq!(full_path, expected.full_path);
    }

    if let Some(event) = events.next() {
        panic!("unexpected event: {event:?}");
    }
}

#[derive(Debug, Default)]
struct ExpectEvent<'a> {
    full_path: PathBuf,
    file_path: &'a str,
    is_dir: bool,
    accessed: bool,
    modified: bool,
    metadata_changed: bool,
    closed_write: bool,
    closed_no_write: bool,
    closed: bool,
    opened: bool,
    deleted: bool,
    moved: bool,
    unmounted: bool,
    file_moved_from: bool,
    file_moved_into: bool,
    file_moved: bool,
    file_created: bool,
    file_deleted: bool,
}

impl PartialEq<ExpectEvent<'_>> for &'_ Event {
    #[rustfmt::skip]
    fn eq(&self, event: &ExpectEvent<'_>) -> bool {
        // Don't want to print the entire event as it's quite big, just print
        // the field that differs.
        assert_eq!(self.file_path(), Path::new(event.file_path), "file_path");
        assert_eq!(self.is_dir(), event.is_dir, "is_dir");
        assert_eq!(self.accessed(), event.accessed, "accessed");
        assert_eq!(self.modified(), event.modified, "modified");
        assert_eq!(self.metadata_changed(), event.metadata_changed, "metadata_changed");
        assert_eq!(self.closed_write(), event.closed_write, "closed_write");
        assert_eq!(self.closed_no_write(), event.closed_no_write, "closed_no_write");
        assert_eq!(self.closed(), event.closed, "closed");
        assert_eq!(self.opened(), event.opened, "opened");
        assert_eq!(self.deleted(), event.deleted, "deleted");
        assert_eq!(self.moved(), event.moved, "moved");
        assert_eq!(self.unmounted(), event.unmounted, "unmounted");
        assert_eq!(self.file_moved_from(), event.file_moved_from, "file_moved_from");
        assert_eq!(self.file_moved_into(), event.file_moved_into, "file_moved_into");
        assert_eq!(self.file_moved(), event.file_moved, "file_moved");
        assert_eq!(self.file_created(), event.file_created, "file_created");
        assert_eq!(self.file_deleted(), event.file_deleted, "file_deleted");
        true
    }
}
