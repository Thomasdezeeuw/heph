//! Filesystem manipulation operations.
//!
//! To open a file ([`AsyncFd`]) use [`open_file`] or [`OpenOptions`].
//!
//! [`AsyncFd`]: crate::fd::AsyncFd

pub mod watch;
#[doc(no_inline)]
pub use watch::Watch;

#[doc(inline)]
pub use a10::fs::{
    create_dir, open_file, remove_dir, remove_file, rename, Advise, Allocate, CreateDir, Delete,
    FileType, Metadata, Open, OpenOptions, Permissions, Rename, Stat, SyncData, Truncate,
};
