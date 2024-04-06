//! Module with read buffer pool.
//!
//! See [`ReadBufPool`].

use std::borrow::{Borrow, BorrowMut};
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut, RangeBounds};
use std::{fmt, io};

use crate::io::{Buf, BufMut};
use crate::Access;

use a10::io::{Buf as _, BufMut as _};

/// A read buffer pool.
///
/// This is a special buffer pool that can only be used in read operations done
/// by the kernel, i.e. no in-memory operations. The buffer pool is actually
/// managed by the kernel in `read(2)` and `recv(2)` like calls. Instead of user
/// space having to select a buffer before issuing the read call, the kernel
/// will select a buffer from the pool when it's ready for reading. This avoids
/// the need to have as many buffers as concurrent read calls.
///
/// As a result of this the returned buffer, [`ReadBuf`], is somewhat limited.
/// For example it can't grow beyond the pool's buffer size. However it can be
/// used in write calls like any other buffer.
#[derive(Clone, Debug)]
pub struct ReadBufPool {
    inner: a10::io::ReadBufPool,
}

impl ReadBufPool {
    /// Create a new buffer pool.
    ///
    /// `pool_size` must be a power of 2, with a maximum of 2^15 (32768).
    /// `buf_size` is the maximum capacity of the buffer. Note that buffer can't
    /// grow beyond this capacity.
    pub fn new<RT>(rt: &RT, pool_size: u16, buf_size: u32) -> io::Result<ReadBufPool>
    where
        RT: Access,
    {
        a10::io::ReadBufPool::new(rt.submission_queue(), pool_size, buf_size)
            .map(|inner| ReadBufPool { inner })
    }

    /// Get a buffer reference to this pool.
    ///
    /// This can only be used in kernel read I/O operations, such as
    /// [`TcpStream::recv`]. However it won't yet select a buffer to use. This
    /// is done by the kernel once it actually has data to write into the
    /// buffer. Before it's used in a read call the returned buffer will be
    /// empty and can't be resized, it's effecitvely useless before a read call.
    ///
    /// [`TcpStream::recv`]: crate::net::TcpStream::recv
    pub fn get(&self) -> ReadBuf {
        ReadBuf {
            inner: self.inner.get(),
        }
    }
}

/// Buffer reference from a [`ReadBufPool`].
///
/// Before a read system call, this will be empty and can't be resized. This is
/// really only useful after reading into it.
///
/// # Notes
///
/// Do **not** use the [`BufMut`] implementation of this buffer to write into
/// it, it's a specialised implementation that is invalid use to outside of the
/// Heph crate.
pub struct ReadBuf {
    inner: a10::io::ReadBuf,
}

impl ReadBuf {
    /// Returns the capacity of the buffer.
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Returns the length of the buffer.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns itself as slice.
    pub fn as_slice(&self) -> &[u8] {
        self
    }

    /// Returns itself as mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self
    }

    /// Truncate the buffer to `len` bytes.
    ///
    /// If the buffer is shorter then `len` bytes this does nothing.
    pub fn truncate(&mut self, len: usize) {
        self.inner.truncate(len);
    }

    /// Clear the buffer.
    ///
    /// # Notes
    ///
    /// This is not the same as returning the buffer to the buffer pool, for
    /// that use [`ReadBuf::release`].
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Remove the bytes in `range` from the buffer.
    ///
    /// # Panics
    ///
    /// This will panic if the `range` is invalid.
    pub fn remove<R>(&mut self, range: R)
    where
        R: RangeBounds<usize>,
    {
        self.inner.remove(range);
    }

    /// Set the length of the buffer to `new_len`.
    ///
    /// # Safety
    ///
    /// The caller must ensure `new_len` bytes are initialised and that
    /// `new_len` is not larger than the buffer's capacity.
    pub unsafe fn set_len(&mut self, new_len: usize) {
        self.inner.set_len(new_len);
    }

    /// Appends `other` to `self`.
    ///
    /// If `self` doesn't have sufficient capacity it will return `Err(())` and
    /// will not append anything.
    #[allow(clippy::result_unit_err)]
    pub fn extend_from_slice(&mut self, other: &[u8]) -> Result<(), ()> {
        self.inner.extend_from_slice(other)
    }

    /// Returns the remaining spare capacity of the buffer.
    pub fn spare_capacity_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        self.inner.spare_capacity_mut()
    }

    /// Release the buffer back to the buffer pool.
    ///
    /// If `self` isn't an allocated buffer this does nothing.
    ///
    /// The buffer can still be used in a `read(2)` system call, it's reset to
    /// the state as if it was just created by calling [`ReadBufPool::get`].
    ///
    /// # Notes
    ///
    /// This is automatically called in the `Drop` implementation.
    pub fn release(&mut self) {
        self.inner.release();
    }
}

/// The implementation for `ReadBuf` is a special one as we don't actually pass
/// a "real" buffer. Instead we pass special flags to the kernel that allows it
/// to select a buffer from the connected [`ReadBufPool`] once the actual read
/// operation starts.
///
/// If the `ReadBuf` is used a second time in a read call this changes as at
/// that point it owns an actual buffer. At that point it will behave more like
/// the `Vec<u8>` implementation is that it only uses the unused capacity, so
/// any bytes already in the buffer will be untouched.
///
/// To revert to the original behaviour of allowing the kernel to select a
/// buffer call [`ReadBuf::release`] first.
///
/// Note that this can **not** be used in vectored I/O as a part of the
/// [`BufMutSlice`] trait.
///
/// [`BufMutSlice`]: crate::io::BufMutSlice
unsafe impl BufMut for ReadBuf {
    unsafe fn parts_mut(&mut self) -> (*mut u8, usize) {
        let (ptr, len) = self.inner.parts_mut();
        (ptr, len as usize)
    }

    unsafe fn update_length(&mut self, n: usize) {
        self.set_len(n);
    }

    fn buffer_group(&self) -> Option<a10::io::BufGroupId> {
        self.inner.buffer_group()
    }

    unsafe fn buffer_init(&mut self, idx: a10::io::BufIdx, n: u32) {
        self.inner.buffer_init(idx, n);
    }

    fn spare_capacity(&self) -> usize {
        self.inner.capacity() - self.inner.len()
    }

    fn has_spare_capacity(&self) -> bool {
        self.inner.capacity() >= self.inner.len()
    }
}

// SAFETY: `ReadBuf` manages the allocation of the bytes once it's assigned a
// buffer, so as long as it's alive, so is the slice of bytes.
unsafe impl Buf for ReadBuf {
    unsafe fn parts(&self) -> (*const u8, usize) {
        let (ptr, len) = self.inner.parts();
        (ptr, len as usize)
    }

    fn as_slice(&self) -> &[u8] {
        self.inner.as_slice()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl Deref for ReadBuf {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for ReadBuf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl AsRef<[u8]> for ReadBuf {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl AsMut<[u8]> for ReadBuf {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}

impl Borrow<[u8]> for ReadBuf {
    fn borrow(&self) -> &[u8] {
        self
    }
}

impl BorrowMut<[u8]> for ReadBuf {
    fn borrow_mut(&mut self) -> &mut [u8] {
        self
    }
}

impl fmt::Debug for ReadBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}
