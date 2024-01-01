//! Watch filesystem changes.
//!
//! Files and directories can be watched using [`Watch`], but see the notes
//! below!
//!
//! # Examples
//!
//! ```
//! # #![feature(never_type)]
//! use std::io;
//! use std::path::{Path, PathBuf};
//!
//! use heph_rt::fs::{watch, Watch};
//! use heph_rt::Access;
//! use heph::actor;
//!
//! async fn actor<RT: Access>(ctx: actor::Context<!, RT>, path: PathBuf) -> io::Result<()> {
//!     // Create our watch.
//!     let mut watch = Watch::new(ctx.runtime_ref())?;
//!
//!     let file_name = Path::new("file.txt");
//!     let file_path = path.join(file_name);
//!
//!     // Watch a directory at `path`, generating events for everything that is
//!     // supported and watch the directory recursively.
//!     watch.watch_directory(path, watch::Interest::ALL, watch::Recursive::All)?;
//!
//!     // Let's pretend another program (or elsewhere in the code) we created a
//!     // file in the watched directory.
//!     std::fs::write(&file_path, b"Hello, World!")?;
//!
//!     // Wait for filesystem events.
//!     let mut events = watch.events().await?;
//!     // Process the event one at a time.
//!     while let Some(event) = events.next() {
//!         // Get the path relative to the watched directory.
//!         let relative_to_dir_path = event.file_path();
//!         assert_eq!(relative_to_dir_path, file_name);
//!         // Or get the full path.
//!         let path = events.path_for(event);
//!         assert_eq!(path, file_path);
//!
//!         // We can also check what happened to the directory, in this case we
//!         // expect a file creation event.
//!         assert!(event.file_created());
//!     }
//!
//!     Ok(())
//! }
//! # _ = actor::<heph_rt::ThreadLocal>; // Silence dead code warnings.
//! ```
//!
//! # Notes
//!
//! This implementation is based on [`inotify(7)`], which has a number of
//! caveats. Most of them are straight from the manual.
//!
//! Events are not generated for files inside a watched directory that are
//! performed via a symbolic link that lies outside the watched directory.
//!
//! Events are only reported that are generated by user-space programs. Thus
//! watching remote filesystems and pseudo-filesystems (e.g. `/proc` or `/sys`)
//! is not supported.
//!
//! No events are generated for modifications made through `mmap(2)`,
//! `msync(2)`, and `munmap(2)`.
//!
//! This API (and `inotify(7)`) work with paths, however this means there is
//! always a race condition between the time the event is generated and the time
//! the event is processed, in which the file at the path can be deleted or
//! renamed, etc.
//!
//! The event queue can overflow. In this case, events are lost. See
//! `/proc/sys/fs/inotify/max_queued_events` for the maximum queue length.
//!
//! If a filesystem is mounted on top of a monitored directory, no event is
//! generated, and no events are generated for objects immediately under the new
//! mount point. If the filesystem is subsequently unmounted, events will
//! subsequently be generated for the directory and the objects it contains.
//!
//! [`inotify(7)`]: https://man7.org/linux/man-pages/man7/inotify.7.html

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::{CString, OsStr, OsString};
use std::mem::{size_of, take};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::path::{Path, PathBuf};
use std::{fmt, io, ptr};

use a10::AsyncFd;
use log::warn;

use crate::access::Access;

/// Filesystem change watch.
///
/// This can be used to watch directories and files for changes. See the [module
/// documentation] for examples and caveats.
///
/// [module documentation]: crate::fs::watch
#[derive(Debug)]
pub struct Watch {
    /// `inotify` file descriptor.
    fd: AsyncFd,
    /// The watch descriptors (wds) and the path to the file or directory they
    /// are watching.
    watching: HashMap<RawFd, PathBufWithNull>,
    /// Buffer for reading events.
    buf: Vec<u8>,
}

/// A valid null terminated [`PathBuf`], encoding is OS specific.
type PathBufWithNull = CString;

impl Watch {
    /// Create a new file system watcher.
    pub fn new<RT>(rt: &RT) -> io::Result<Watch>
    where
        RT: Access,
    {
        let fd = syscall!(inotify_init1(libc::IN_CLOEXEC))?;
        // SAFETY: just create the fd, so it's valid.
        let fd = unsafe { AsyncFd::from_raw_fd(fd, rt.submission_queue()) };
        let watching = HashMap::new();
        let buf = Vec::new();
        Ok(Watch { fd, watching, buf })
    }

    /// Watch `dir`ectory.
    ///
    /// If `recursive` is `Recursive::All` it recursively watches all
    /// directories in `dir`.
    pub fn watch_directory(
        &mut self,
        dir: PathBuf,
        interest: Interest,
        recursive: Recursive,
    ) -> io::Result<()> {
        // Watch the path only if it is a directory, otherwise return an error.
        self._watch(dir.clone(), interest.0 | libc::IN_ONLYDIR)?;

        if let Recursive::All = recursive {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let path = entry.path();
                    self.watch_directory(path, interest, Recursive::All)?;
                }
            }
        }
        Ok(())
    }

    /// Watch `file`.
    pub fn watch_file(&mut self, file: PathBuf, interest: Interest) -> io::Result<()> {
        self._watch(file, interest.0)
    }

    fn _watch(&mut self, path: PathBuf, mask: u32) -> io::Result<()> {
        let path: PathBufWithNull =
            unsafe { CString::from_vec_unchecked(OsString::from(path).into_encoded_bytes()) };
        let mask = mask
            // Don't follow symbolic links.
            | libc::IN_DONT_FOLLOW
            // When files are moved out of a watched directory don't generate
            // events for them.
            | libc::IN_EXCL_UNLINK
            // Instead of replacing a watch combine the watched events.
            | libc::IN_MASK_ADD;
        let fd = self.fd.as_fd().as_raw_fd();
        let wd = syscall!(inotify_add_watch(fd, path.as_ptr(), mask))?;
        _ = self.watching.insert(wd, path);
        Ok(())
    }

    /// Wait for filesystem events.
    pub async fn events<'w>(&'w mut self) -> io::Result<Events<'w>> {
        let mut buf = take(&mut self.buf);
        buf.clear();
        // `inotify_event` is 16 bytes + the path name, which has a max of 255,
        // so this can hold at least a couple of events.
        buf.reserve(1024);
        // TODO: handle 0 read?
        self.buf = self.fd.read(buf).await?;
        Ok(Events {
            watching: &mut self.watching,
            buf: &self.buf,
            processed: 0,
        })
    }
}

impl AsFd for Watch {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Iterator behind [`Watch::events`].
pub struct Events<'w> {
    /// `Watch::Watching`.
    watching: &'w mut HashMap<RawFd, PathBufWithNull>,
    /// `Watch::buf`.
    buf: &'w [u8],
    /// Number of bytes processed in the `watch.buf`fer.
    processed: usize,
}

impl<'w> Events<'w> {
    /// Returns the path for `event`.
    ///
    /// # Notes
    ///
    /// Internally we keep track of which paths are watched. If a path is
    /// deleted it is removed from this internal bookkeeping, meaning that this
    /// will return the same value as [`Event::file_path`] (which is empty for
    /// files).
    ///
    /// To ensure that you always get the full path call this method **before**
    /// calling [`Events::next`] when processing events.
    pub fn path_for<'a>(&'a self, event: &'a Event) -> Cow<'a, Path> {
        match self.watched_path(&event.event.wd) {
            Some(path) if event.path.is_empty() => Cow::Borrowed(path),
            Some(path) => Cow::Owned(path.join(event.file_path())),
            None => Cow::Borrowed(event.file_path()),
        }
    }

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn watched_path<'a>(&'a self, wd: &RawFd) -> Option<&'a Path> {
        self.watching.get(wd).map(move |path| {
            // SAFETY: the path was passed to us as a valid `PathBuf`, so it
            // must be a valid `Path`.
            let path = unsafe { OsStr::from_encoded_bytes_unchecked(path.as_bytes()) };
            Path::new(path)
        })
    }
}

impl<'w> Iterator for Events<'w> {
    type Item = &'w Event;

    fn next(&mut self) -> Option<Self::Item> {
        if self.processed + size_of::<libc::inotify_event>() > self.buf.len() {
            return None;
        }

        // SAFETY: the kernel ensures that the read bytes are `inotify_event` a
        // structure followed by `event.len` bytes that make up the pathname.
        let event: &'w libc::inotify_event =
            unsafe { &*self.buf.as_ptr().add(self.processed).cast() };
        // Ensure that we don't process the same event twice.
        self.processed += size_of::<libc::inotify_event>() + event.len as usize;

        // `IN_IGNORED` means the file is no longer watched. An event before
        // this should contain the information why (e.g. the file was deleted).
        if event.mask & libc::IN_IGNORED != 0 {
            _ = self.watching.remove(&event.wd);
            return self.next();
        }

        if event.mask & libc::IN_Q_OVERFLOW != 0 {
            warn!("inotify event queue overflowed");
            return self.next();
        }

        // SAFETY: we determine the length based on the kernel's information, so
        // it should be valid.
        debug_assert!(self.buf.len() >= self.processed);
        let event: &'w Event =
            unsafe { &*ptr::from_raw_parts(ptr::from_ref(event).cast(), event.len as usize) };

        Some(event)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<'w> ExactSizeIterator for Events<'w> {
    fn len(&self) -> usize {
        let mut count = 0;
        let mut bytes_processed = self.processed;
        loop {
            if bytes_processed + size_of::<libc::inotify_event>() > self.buf.len() {
                return count;
            }

            // SAFETY: the kernel ensures that the read bytes are `inotify_event` a
            // structure followed by `event.len` bytes that make up the pathname.
            let event: &'w libc::inotify_event =
                unsafe { &*self.buf.as_ptr().add(bytes_processed).cast() };
            // Ensure that we don't process the same event twice.
            bytes_processed += size_of::<libc::inotify_event>() + event.len as usize;

            // For these two events we don't return an event to the user, we
            // handle them internally.
            if event.mask & (libc::IN_IGNORED | libc::IN_Q_OVERFLOW) != 0 {
                continue;
            }
            count += 1;
        }
    }
}

impl<'w> fmt::Debug for Events<'w> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Events")
            .field("count", &self.len())
            .finish()
    }
}

impl<'w> Drop for Events<'w> {
    fn drop(&mut self) {
        while self.next().is_some() { /* Process `IN_IGNORED` events. */ }
    }
}

/// Event that represent a file system change.
pub struct Event {
    event: libc::inotify_event,
    path: [u8],
}

impl Event {
    /// Path to the file within the watched directory.
    ///
    /// This will only be non-empty for events triggered by files/directories in
    /// watched directories. It be empty for events on watched files and
    /// directories themselves.
    ///
    /// See [`Events::path_for`] to the get the full path for watched files and
    /// directories.
    pub fn file_path(&self) -> &Path {
        // The path can contain null bytes as padding to aligned the
        // `inotify_event`s.
        let end = self
            .path
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(self.path.len());
        let path = &self.path[..end];
        // SAFETY: the path comes from the OS, so it should be a valid OS
        // string.
        let path = unsafe { OsStr::from_encoded_bytes_unchecked(path) };
        Path::new(path)
    }

    // Getters for the events.
    bit_checks!(self.event.mask);
}

/// Macro to create functions to check bits set.
macro_rules! bit_checks {
    ( $self: ident . $field: ident . $field2: ident ) => {
        bit_checks!(
            $self.$field.$field2,
            /// Return true if the subject of this event is a directory.
            is_dir, IN_ISDIR;
            /// Returns true if:
            ///  * the watched file was accessed, or
            ///  * a file within a watched directory was accessed.
            accessed, IN_ACCESS;
            /// Returns true if:
            ///  * the watched file was modified, or
            ///  * a file within a watched directory was modified.
            modified, IN_MODIFY;
            /// Returns true if:
            ///  * the watched file had its metadata (attributes) changed,
            ///  * a file within a watched directory had its metadata changed, or
            ///  * the watched directory had its metadata changed.
            metadata_changed, IN_ATTRIB;
            /// Returns true if:
            ///  * the watched file, opened for writing, was closed, or
            ///  * a file, opened for writing, within a watched directory was closed.
            ///
            ///  # Notes
            ///
            ///  See [`closed`] for a check that ignores whether or not the file
            ///  was opened for writing or not.
            ///
            ///  [`closed`]: Self::closed
            closed_write, IN_CLOSE_WRITE;
            /// Returns true if:
            ///  * the watched file, not opened for writing, was closed, or
            ///  * a file, not opened for writing, within a watched directory was closed.
            ///  * the watched directory was closed.
            ///
            ///  # Notes
            ///
            ///  See [`closed`] for a check that ignores whether or not the file
            ///  was opened for writing or not.
            ///
            ///  [`closed`]: Self::closed
            closed_no_write, IN_CLOSE_NOWRITE;
            /// Returns true if:
            ///  * the watched file was closed, or
            ///  * a file within a watched directory was closed.
            ///  * the watched directory was closed.
            closed, IN_CLOSE;
            /// Returns true if:
            ///  * the watched file was opened.
            ///  * a file within a watched directory was opened, or
            ///  * the watched directory was opened.
            opened, IN_OPEN;
            /// Returns true if:
            ///  * the watched file was deleted.
            ///  * the watched directory was deleted.
            deleted, IN_DELETE_SELF;
            /// Returns true if:
            ///  * the watched file was moved.
            ///  * the watched directory was moved.
            ///
            /// # Notes
            ///
            /// If a file is moved to another file system this will not trigger
            /// this, but instead trigger [`deleted`].
            ///
            /// [`deleted`]: Self::deleted
            moved, IN_MOVE_SELF;
            /// Returns true if the filesystem containing the watched file or
            /// directory was unmounted.
            unmounted, IN_UNMOUNT;

            // Directory only.

            /// Returns true if:
            ///  * a file within a watched directory was moved out of the watched directory.
            file_moved_from, IN_MOVED_FROM;
            /// Returns true if:
            ///  * a file within a watched directory was moved into the watched directory.
            file_moved_into, IN_MOVED_TO;
            /// Returns true if:
            ///  * a file within a watched directory was moved (into or of out of the watched directory).
            file_moved, IN_MOVE;
            /// Returns true if:
            ///  * a file within a watched directory was created.
            file_created, IN_CREATE;
            /// Returns true if:
            ///  * a file within a watched directory was deleted.
            file_deleted, IN_DELETE;
        );
    };
    (
        $self: ident . $field: ident . $field2: ident,
        $( $(#[$meta: meta])* $fn_name: ident, $bit: ident ; )+
    ) => {
        $(
        $( #[$meta] )*
        #[doc(alias = $bit)]
        pub const fn $fn_name(&$self) -> bool {
            $self.$field.$field2 & libc::$bit != 0
        }
        )+

        fn fmt_event_fields(&self, f: &mut fmt::DebugStruct<'_, '_>) {
            $(
            _ = f.field(stringify!($fn_name), &self.$fn_name());
            )+
        }
    };
}

use bit_checks;

#[allow(clippy::missing_fields_in_debug)] // `path` is included as `file_path`.
impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_struct("Event");
        _ = f
            .field("wd", &self.event.wd)
            .field("cookie", &self.event.cookie)
            .field("file_path", &self.file_path())
            .field("mask", &self.event.mask);
        self.fmt_event_fields(&mut f);
        f.finish()
    }
}

/// What kind of filesystem changes we're interested in monitering.
#[derive(Copy, Clone, Debug)]
pub struct Interest(u32);

impl Interest {
    /// Watch everything.
    #[doc(alias = "IN_ALL_EVENTS")]
    pub const ALL: Interest = Interest(libc::IN_ALL_EVENTS);

    /// File was accessed, e.g. read.
    #[doc(alias = "IN_ACCESS")]
    pub const ACCESS: Interest = Interest(libc::IN_ACCESS);

    /// File was modified, e.g. written.
    #[doc(alias = "IN_MODIFY")]
    pub const MODIFY: Interest = Interest(libc::IN_MODIFY);

    /// Metadata or attribute changed, e.g. permissions where changed.
    #[doc(alias = "IN_ATTRIB")]
    pub const METADATA: Interest = Interest(libc::IN_ATTRIB);

    /// File opened for writing was closed.
    #[doc(alias = "IN_CLOSE_WRITE")]
    pub const CLOSE_WRITE: Interest = Interest(libc::IN_CLOSE_WRITE);

    /// File or directory not opened for writing was closed.
    #[doc(alias = "IN_CLOSE_NOWRITE")]
    pub const CLOSE_NOWRITE: Interest = Interest(libc::IN_CLOSE_NOWRITE);

    /// Combination of [`Interest::CLOSE_WRITE`] and [`Interest::CLOSE_NOWRITE`]
    /// to get all closing events.
    #[doc(alias = "IN_CLOSE")]
    pub const CLOSE: Interest = Interest(libc::IN_CLOSE);

    /// File or directory was opened.
    #[doc(alias = "IN_OPEN")]
    pub const OPEN: Interest = Interest(libc::IN_OPEN);

    /// A file was moved out of the watched directory was renamed.
    #[doc(alias = "IN_MOVED_FROM")]
    pub const MOVE_FROM: Interest = Interest(libc::IN_MOVED_FROM);

    /// A file was moved into the watched directory.
    #[doc(alias = "IN_MOVED_TO")]
    pub const MOVE_INTO: Interest = Interest(libc::IN_MOVED_TO);

    /// Combination of [`Interest::MOVE_FROM`] and [`Interest::MOVE_INTO`] to
    /// get all closing events.
    #[doc(alias = "IN_MOVE")]
    pub const MOVE: Interest = Interest(libc::IN_MOVE);

    /// File or directory was created in a watched directory.
    #[doc(alias = "IN_CREATE")]
    pub const CREATE: Interest = Interest(libc::IN_CREATE);

    /// File or directory was deleted from a watched directory.
    #[doc(alias = "IN_DELETE")]
    pub const DELETE: Interest = Interest(libc::IN_DELETE);

    /// File or directory itself was deleted.
    ///
    /// # Notes
    ///
    /// This event also occurs if an object is moved to another filesystem,
    /// since a move in effect copies the file to the other filesystem and then
    /// deletes it from the original filesystem.
    #[doc(alias = "IN_DELETE_SELF")]
    pub const DELETE_SELF: Interest = Interest(libc::IN_DELETE_SELF);

    /// File or directory itself was moved.
    #[doc(alias = "IN_MOVE_SELF")]
    pub const MOVE_SELF: Interest = Interest(libc::IN_MOVE_SELF);

    /// Add `other` interest to this interest.
    #[must_use]
    pub const fn add(self, other: Interest) -> Interest {
        Interest(self.0 | other.0)
    }
}

/// How to recursively watch a directory.
#[derive(Copy, Clone, Debug)]
pub enum Recursive {
    /// Don't watch recursively.
    ///
    /// Only get events for the files and directories directly in the watched
    /// directory.
    ///
    /// # Examples
    ///
    /// The following illustraties which files and directories and watches and
    /// which aren't.
    ///
    /// ```text
    /// # While watching `src/`.
    /// src/main.rs   # Watched
    /// src/fs        # Watched.
    /// src/fs/mod.rs # Not watched.
    /// ```
    No,
    /// Watch recursively.
    ///
    /// This will walk the entire directory tree to create watches for each
    /// sub-directory. This may take some time for large directories.
    ///
    /// # Notes
    ///
    /// `Watch` doesn't automatically watch directories added to a watch
    /// directory. To watch new directories you have to manually setup a watch
    /// for the newly created directory.
    All,
}
