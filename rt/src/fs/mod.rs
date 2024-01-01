//! Filesystem manipulation operations.
//!
//! To open a [`File`] use [`File::open`] or [`OpenOptions`].

use std::os::fd::{AsFd, BorrowedFd};
use std::path::PathBuf;
use std::time::SystemTime;
use std::{fmt, io};

use a10::{AsyncFd, Extract};

use crate::access::Access;
use crate::io::futures::{
    Read, ReadN, ReadNVectored, ReadVectored, Write, WriteAll, WriteAllVectored, WriteVectored,
};
use crate::io::{impl_read, impl_write, Buf, BufMut, BufMutSlice, BufSlice, BufWrapper};
use crate::wakers::NoRing;

pub mod watch;
#[doc(no_inline)]
pub use watch::Watch;

/// Access to an open file on the filesystem.
///
/// A`File` can be read and/or written depending on what options it was opened
/// with.
pub struct File {
    fd: AsyncFd,
}

impl File {
    /// Open a file in read-only mode.
    pub async fn open<RT>(rt: &RT, path: PathBuf) -> io::Result<File>
    where
        RT: Access,
    {
        NoRing(OpenOptions::new().read().open(rt, path)).await
    }

    /// Opens a file in write-only mode.
    ///
    /// This will create a new file if it does not exist, and will truncate it
    /// if it does.
    pub async fn create<RT>(rt: &RT, path: PathBuf) -> io::Result<File>
    where
        RT: Access,
    {
        NoRing(
            OpenOptions::new()
                .write()
                .create()
                .truncate()
                .open(rt, path),
        )
        .await
    }

    /// Returns the default `OpenOptions`.
    pub const fn options() -> OpenOptions {
        OpenOptions::new()
    }

    /// Converts a [`std::fs::File`] to a [`heph_rt::fs::File`].
    ///
    /// [`heph_rt::fs::File`]: File
    pub fn from_std<RT>(rt: &RT, file: std::fs::File) -> File
    where
        RT: Access,
    {
        File {
            fd: AsyncFd::new(file.into(), rt.submission_queue()),
        }
    }

    /// Creates a new independently owned `File` that shares the same
    /// underlying file descriptor as the existing `File`.
    pub fn try_clone(&self) -> io::Result<File> {
        Ok(File {
            fd: self.fd.try_clone()?,
        })
    }

    /// Read bytes from the file at `offset`, writing them into `buf`.
    ///
    /// The current file cursor is not affected by this function. This means
    /// that a call `read_at(buf, 1024)` with a buffer of 1kb will **not**
    /// continue reading at 2kb in the next call to `read`.
    pub async fn read_at<B: BufMut>(&self, buf: B, offset: u64) -> io::Result<B> {
        Read(self.fd.read_at(BufWrapper(buf), offset)).await
    }

    /// Read at least `n` bytes from the file at `offset`, writing them into
    /// `buf`.
    ///
    /// Returns [`io::ErrorKind::UnexpectedEof`] if less than `n` bytes could be
    /// read.
    ///
    /// The current file cursor is not affected by this function.
    pub async fn read_n_at<B: BufMut>(&self, buf: B, offset: u64, n: usize) -> io::Result<B> {
        debug_assert!(
            buf.spare_capacity() >= n,
            "called `File::read_n_at` with a buffer smaller than `n`",
        );
        ReadN(self.fd.read_n_at(BufWrapper(buf), offset, n)).await
    }

    /// Read bytes from the file, writing them into `bufs`.
    ///
    /// The current file cursor is not affected by this function.
    pub async fn read_vectored_at<B: BufMutSlice<N>, const N: usize>(
        &self,
        bufs: B,
        offset: u64,
    ) -> io::Result<B> {
        ReadVectored(self.fd.read_vectored_at(BufWrapper(bufs), offset)).await
    }

    /// Read at least `n` bytes from the file at `offset`, writing them into `bufs`.
    ///
    /// The current file cursor is not affected by this function.
    pub async fn read_n_vectored_at<B: BufMutSlice<N>, const N: usize>(
        &self,
        bufs: B,
        offset: u64,
        n: usize,
    ) -> io::Result<B> {
        debug_assert!(
            bufs.total_spare_capacity() >= n,
            "called `File::read_n_vectored_at` with buffers smaller than `n`"
        );
        ReadNVectored(self.fd.read_n_vectored_at(BufWrapper(bufs), offset, n)).await
    }

    /// Write the bytes in `buf` to the file at `offset`.
    ///
    /// Returns the number of bytes written. This may we fewer than the length
    /// of `buf`. To ensure that all bytes are written use
    /// [`File::write_all_at`].
    pub async fn write_at<B: Buf>(&self, buf: B, offset: u64) -> io::Result<(B, usize)> {
        Write(self.fd.write_at(BufWrapper(buf), offset).extract()).await
    }

    /// Write the all bytes in `buf` to the file at `offset`.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub async fn write_all_at<B: Buf>(&self, buf: B, offset: u64) -> io::Result<B> {
        WriteAll(self.fd.write_all_at(BufWrapper(buf), offset).extract()).await
    }

    /// Write the bytes in `bufs` to the file at `offset`.
    ///
    /// Return the number of bytes written. This may we fewer than the length of
    /// `bufs`. To ensure that all bytes are written use
    /// [`File::write_vectored_all_at`].
    pub async fn write_vectored_at<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
        offset: u64,
    ) -> io::Result<(B, usize)> {
        WriteVectored(
            self.fd
                .write_vectored_at(BufWrapper(bufs), offset)
                .extract(),
        )
        .await
    }

    /// Write the all bytes in `bufs` to the file at `offset`.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub async fn write_vectored_all_at<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
        offset: u64,
    ) -> io::Result<B> {
        WriteAllVectored(
            self.fd
                .write_all_vectored_at(BufWrapper(bufs), offset)
                .extract(),
        )
        .await
    }

    /// Sync all OS-internal metadata to disk.
    ///
    /// # Notes
    ///
    /// Any uncompleted writes may not be synced to disk.
    pub async fn sync_all(&self) -> io::Result<()> {
        NoRing(self.fd.sync_all()).await
    }

    /// This function is similar to [`sync_all`], except that it may not
    /// synchronize file metadata to the filesystem.
    ///
    /// This is intended for use cases that must synchronize content, but don’t
    /// need the metadata on disk. The goal of this method is to reduce disk
    /// operations.
    ///
    /// [`sync_all`]: AsyncFd::sync_all
    ///
    /// # Notes
    ///
    /// Any uncompleted writes may not be synced to disk.
    pub async fn sync_data(&self) -> io::Result<()> {
        NoRing(self.fd.sync_data()).await
    }

    /// Retrieve metadata about the file.
    pub async fn metadata(&self) -> io::Result<Metadata> {
        NoRing(self.fd.metadata())
            .await
            .map(|m| Metadata { inner: *m })
    }

    /// Predeclare an access pattern for file data.
    ///
    /// Announce an intention to access file data in a specific pattern in the
    /// future, thus allowing the kernel to perform appropriate optimizations.
    ///
    /// The advice applies to a (not necessarily existent) region starting at
    /// offset and extending for len bytes (or until the end of the file if len
    /// is 0). The advice is not binding; it merely constitutes an expectation
    /// on behalf of the application.
    pub async fn advise(&self, offset: u64, length: u32, advice: Advice) -> io::Result<()> {
        NoRing(self.fd.advise(offset, length, advice.as_libc())).await
    }

    /// Manipulate file space.
    ///
    /// Manipulate the allocated disk space for the file referred for the byte
    /// range starting at `offset` and continuing for `length` bytes.
    pub async fn allocate(&self, offset: u64, length: u32, mode: AllocateMode) -> io::Result<()> {
        NoRing(self.fd.allocate(offset, length, mode.as_libc())).await
    }
}

impl_read!(File, &File);
impl_write!(File, &File);

impl AsFd for File {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fd.fmt(f)
    }
}

/// Options used to configure how a [`File`] is opened.
#[derive(Clone)]
#[must_use = "no file is opened until `fs::OpenOptions::open` or `open_temp_file` is called"]
pub struct OpenOptions {
    inner: a10::fs::OpenOptions,
}

impl OpenOptions {
    /// Empty `OpenOptions`, has reading enabled by default.
    pub const fn new() -> OpenOptions {
        OpenOptions {
            inner: a10::fs::OpenOptions::new(),
        }
    }

    /// Enable read access.
    ///
    /// Note that read access is already enabled by default, so this is only
    /// useful if you called [`OpenOptions::write_only`] and want to enable read
    /// access as well.
    pub const fn read(self) -> Self {
        OpenOptions {
            inner: self.inner.read(),
        }
    }

    /// Enable write access.
    pub const fn write(self) -> Self {
        OpenOptions {
            inner: self.inner.write(),
        }
    }

    /// Only enable write access, disabling read access.
    pub const fn write_only(self) -> Self {
        OpenOptions {
            inner: self.inner.write_only(),
        }
    }

    /// Set writing to append only mode.
    ///
    /// # Notes
    ///
    /// This requires [writing access] to be enabled.
    ///
    /// [writing access]: OpenOptions::write
    pub const fn append(self) -> Self {
        OpenOptions {
            inner: self.inner.append(),
        }
    }

    /// Truncate the file if it exists.
    pub const fn truncate(self) -> Self {
        OpenOptions {
            inner: self.inner.truncate(),
        }
    }

    /// If the file doesn't exist create it.
    pub const fn create(self) -> Self {
        OpenOptions {
            inner: self.inner.create(),
        }
    }

    /// Force a file to be created, failing if a file already exists.
    ///
    /// This options implies [`OpenOptions::create`].
    pub const fn create_new(self) -> Self {
        OpenOptions {
            inner: self.inner.create_new(),
        }
    }

    /// Write operations on the file will complete according to the requirements
    /// of synchronized I/O *data* integrity completion.
    ///
    /// By the time `write(2)` (and similar) return, the output data has been
    /// transferred to the underlying hardware, along with any file metadata
    /// that would be required to retrieve that data (i.e., as though each
    /// `write(2)` was followed by a call to `fdatasync(2)`).
    pub const fn data_sync(self) -> Self {
        OpenOptions {
            inner: self.inner.data_sync(),
        }
    }

    /// Write operations on the file will complete according to the requirements
    /// of synchronized I/O *file* integrity completion (by contrast with the
    /// synchronized I/O data integrity completion provided by
    /// [`OpenOptions::data_sync`].)
    ///
    /// By the time `write(2)` (or similar) returns, the output data and
    /// associated file metadata have been transferred to the underlying
    /// hardware (i.e., as though each `write(2)` was followed by a call to
    /// `fsync(2)`).
    pub const fn sync(self) -> Self {
        OpenOptions {
            inner: self.inner.sync(),
        }
    }

    /// Try to minimize cache effects of the I/O to and from this file.
    ///
    /// File I/O is done directly to/from user-space buffers. This uses the
    /// `O_DIRECT` flag which on its own makes an effort to transfer data
    /// synchronously, but does not give the guarantees of the `O_SYNC` flag
    /// ([`OpenOptions::sync`]) that data and necessary metadata are
    /// transferred. To guarantee synchronous I/O, `O_SYNC` must be used in
    /// addition to `O_DIRECT`.
    pub const fn direct(self) -> Self {
        OpenOptions {
            inner: self.inner.direct(),
        }
    }

    /// Create an unnamed temporary regular file. The `dir` argument specifies a
    /// directory; an unnamed inode will be created in that directory's
    /// filesystem. Anything written to the resulting file will be lost when the
    /// last file descriptor is closed, unless the file is given a name.
    ///
    /// [`OpenOptions::write`] must be set. The `linkat(2)` system call can be
    /// used to make the temporary file permanent.
    pub async fn open_temp_file<RT>(self, rt: &RT, dir: PathBuf) -> io::Result<File>
    where
        RT: Access,
    {
        NoRing(self.inner.open_temp_file(rt.submission_queue(), dir))
            .await
            .map(|fd| File { fd })
    }

    /// Open `path`.
    pub async fn open<RT>(self, rt: &RT, path: PathBuf) -> io::Result<File>
    where
        RT: Access,
    {
        NoRing(self.inner.open(rt.submission_queue(), path))
            .await
            .map(|fd| File { fd })
    }
}

impl fmt::Debug for OpenOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

/// Metadata information about a file.
///
/// See [`File::metadata`].
pub struct Metadata {
    inner: a10::fs::Metadata,
}

impl Metadata {
    /// Returns the file type for this metadata.
    pub const fn file_type(&self) -> FileType {
        FileType(self.inner.file_type())
    }

    /// Returns `true` if this represents a directory.
    pub const fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Returns `true` if this represents a file.
    pub const fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Returns `true` if this represents a symbolic link.
    pub const fn is_symlink(&self) -> bool {
        self.inner.is_symlink()
    }

    /// Returns the size of the file, in bytes, this metadata is for.
    #[allow(clippy::len_without_is_empty)] // Doesn't make sense.
    pub const fn len(&self) -> u64 {
        self.inner.len()
    }

    /// The "preferred" block size for efficient filesystem I/O.
    pub const fn block_size(&self) -> u32 {
        self.inner.block_size()
    }

    /// Returns the permissions of the file this metadata is for.
    pub const fn permissions(&self) -> Permissions {
        Permissions(self.inner.permissions())
    }

    /// Returns the time this file was last modified.
    pub fn modified(&self) -> SystemTime {
        self.inner.modified()
    }

    /// Returns the time this file was last accessed.
    ///
    /// # Notes
    ///
    /// It's possible to disable keeping track of this access time, which makes
    /// this function return an invalid value.
    pub fn accessed(&self) -> SystemTime {
        self.inner.accessed()
    }

    /// Returns the time this file was created.
    pub fn created(&self) -> SystemTime {
        self.inner.created()
    }
}

impl fmt::Debug for Metadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

/// A structure representing a type of file with accessors for each file type.
///
/// See [`Metadata`].
#[derive(Copy, Clone)]
pub struct FileType(a10::fs::FileType);

impl FileType {
    /// Returns `true` if this represents a directory.
    pub const fn is_dir(&self) -> bool {
        self.0.is_dir()
    }

    /// Returns `true` if this represents a file.
    pub const fn is_file(&self) -> bool {
        self.0.is_file()
    }

    /// Returns `true` if this represents a symbolic link.
    pub const fn is_symlink(&self) -> bool {
        self.0.is_symlink()
    }

    /// Returns `true` if this represents a socket.
    pub const fn is_socket(&self) -> bool {
        self.0.is_socket()
    }

    /// Returns `true` if this represents a block device.
    pub const fn is_block_device(&self) -> bool {
        self.0.is_block_device()
    }

    /// Returns `true` if this represents a character device.
    pub const fn is_character_device(&self) -> bool {
        self.0.is_character_device()
    }

    /// Returns `true` if this represents a named fifo pipe.
    pub const fn is_named_pipe(&self) -> bool {
        self.0.is_named_pipe()
    }
}

impl fmt::Debug for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Access permissions.
///
/// See [`Metadata`].
#[derive(Copy, Clone)]
pub struct Permissions(a10::fs::Permissions);

impl Permissions {
    /// Return `true` if the owner has read permission.
    pub const fn owner_can_read(&self) -> bool {
        self.0.owner_can_read()
    }

    /// Return `true` if the owner has write permission.
    pub const fn owner_can_write(&self) -> bool {
        self.0.owner_can_write()
    }

    /// Return `true` if the owner has execute permission.
    pub const fn owner_can_execute(&self) -> bool {
        self.0.owner_can_execute()
    }

    /// Return `true` if the group the file belongs to has read permission.
    pub const fn group_can_read(&self) -> bool {
        self.0.group_can_read()
    }

    /// Return `true` if the group the file belongs to has write permission.
    pub const fn group_can_write(&self) -> bool {
        self.0.group_can_write()
    }

    /// Return `true` if the group the file belongs to has execute permission.
    pub const fn group_can_execute(&self) -> bool {
        self.0.group_can_execute()
    }

    /// Return `true` if others have read permission.
    pub const fn others_can_read(&self) -> bool {
        self.0.others_can_read()
    }

    /// Return `true` if others have write permission.
    pub const fn others_can_write(&self) -> bool {
        self.0.others_can_write()
    }

    /// Return `true` if others have execute permission.
    pub const fn others_can_execute(&self) -> bool {
        self.0.others_can_execute()
    }
}

impl fmt::Debug for Permissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Advice passed to [`File::advise`].
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Advice {
    /// Indicates that the application has no advice to give about its access
    /// pattern for the specified data.  If no advice is given for an open file,
    /// this is the default assumption.
    Normal,
    /// The specified data will be accessed in random order.
    Random,
    /// The application expects to access the specified data sequentially (with
    /// lower offsets read before higher ones).
    Sequential,
    /// The specified data will be accessed in the near future.
    WillNeed,
    /// The specified data will not be accessed in the near future.
    DontNeed,
    /// The specified data will be accessed only once.
    Noreuse,
}

impl Advice {
    const fn as_libc(self) -> libc::c_int {
        match self {
            Advice::Normal => libc::POSIX_FADV_NORMAL,
            Advice::Random => libc::POSIX_FADV_RANDOM,
            Advice::Sequential => libc::POSIX_FADV_SEQUENTIAL,
            Advice::WillNeed => libc::POSIX_FADV_WILLNEED,
            Advice::DontNeed => libc::POSIX_FADV_DONTNEED,
            Advice::Noreuse => libc::POSIX_FADV_NOREUSE,
        }
    }
}

/// Allocation mode passed to [`File::allocate`].
///
/// # Notes
///
/// The availability of these operations differ per Linux version **and** file
/// system used, see the [`fallocate(2)`] man page.
///
/// [`fallocate(2)`]: https://www.man7.org/linux/man-pages/man2/fallocate.2.html
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AllocateMode {
    /// Initiase range to zero.
    InitRange,
    /// Initiase range to zero, but don't change the file size.
    InitRangeKeepSize,
    /// Remove the range from the file.
    RemoveRange,
    /// Zero the range from the file.
    ZeroRange,
    /// Insert a range into the file.
    InsertRange,
}

impl AllocateMode {
    const fn as_libc(self) -> libc::c_int {
        match self {
            AllocateMode::InitRange => 0,
            AllocateMode::InitRangeKeepSize => libc::FALLOC_FL_KEEP_SIZE,
            AllocateMode::RemoveRange => libc::FALLOC_FL_COLLAPSE_RANGE,
            AllocateMode::ZeroRange => libc::FALLOC_FL_ZERO_RANGE,
            AllocateMode::InsertRange => libc::FALLOC_FL_INSERT_RANGE,
        }
    }
}

/// Creates a new, empty directory.
pub async fn create_dir<RT>(rt: &RT, path: PathBuf) -> io::Result<()>
where
    RT: Access,
{
    NoRing(a10::fs::create_dir(rt.submission_queue(), path)).await
}

/// Rename a file or directory to a new name.
pub async fn rename<RT>(rt: &RT, from: PathBuf, to: PathBuf) -> io::Result<()>
where
    RT: Access,
{
    NoRing(a10::fs::rename(rt.submission_queue(), from, to)).await
}

/// Remove a file.
pub async fn remove_file<RT>(rt: &RT, path: PathBuf) -> io::Result<()>
where
    RT: Access,
{
    NoRing(a10::fs::remove_file(rt.submission_queue(), path)).await
}

/// Remove a directory.
pub async fn remove_dir<RT>(rt: &RT, path: PathBuf) -> io::Result<()>
where
    RT: Access,
{
    NoRing(a10::fs::remove_dir(rt.submission_queue(), path)).await
}
