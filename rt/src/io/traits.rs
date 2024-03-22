use std::cmp::min;
use std::future::Future;
use std::io::{self, Empty, Sink};
use std::{mem, slice};

use crate::io::{Buf, BufMut, BufMutSlice, BufSlice};

/// Asynchronously reading bytes from a source.
pub trait Read {
    /// Read bytes, writing them into `buf`.
    ///
    /// # Notes
    ///
    /// The caller must always check if at least one byte was read as reading
    /// zero bytes is an indication that no more bytes can be read (we've hit
    /// the end of the file, `EOF`). Failing to do so can result in an infinite
    /// loop.
    fn read<B: BufMut>(&mut self, buf: B) -> impl Future<Output = io::Result<B>>;

    /// Read at least `n` bytes, writing them into `buf`.
    ///
    /// This returns [`io::ErrorKind::UnexpectedEof`] if less than `n` bytes
    /// could be read.
    ///
    /// [`io::ErrorKind::UnexpectedEof`]: std::io::ErrorKind::UnexpectedEof
    fn read_n<B: BufMut>(&mut self, buf: B, n: usize) -> impl Future<Output = io::Result<B>>;

    /// Determines if this `Read`er has an efficient `read_vectored`
    /// implementation.
    ///
    /// If a `Read`er does not override the default `read_vectored`
    /// implementation, code using it may want to avoid the method all together
    /// and coalesce writes into a single buffer for higher performance.
    ///
    /// The default implementation returns `false`.
    fn is_read_vectored(&self) -> bool {
        false
    }

    /// Read bytes, writing them into `bufs`.
    fn read_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> impl Future<Output = io::Result<B>>;

    /// Read at least `n` bytes, writing them into `bufs`.
    fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
        n: usize,
    ) -> impl Future<Output = io::Result<B>>;
}

impl<T: Read> Read for &mut T {
    async fn read<B: BufMut>(&mut self, buf: B) -> io::Result<B> {
        (**self).read(buf).await
    }

    async fn read_n<B: BufMut>(&mut self, buf: B, n: usize) -> io::Result<B> {
        (**self).read_n(buf, n).await
    }

    fn is_read_vectored(&self) -> bool {
        (**self).is_read_vectored()
    }

    async fn read_vectored<B: BufMutSlice<N>, const N: usize>(&mut self, bufs: B) -> io::Result<B> {
        (**self).read_vectored(bufs).await
    }

    async fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
        n: usize,
    ) -> io::Result<B> {
        (**self).read_n_vectored(bufs, n).await
    }
}

impl<T: Read> Read for Box<T> {
    async fn read<B: BufMut>(&mut self, buf: B) -> io::Result<B> {
        (**self).read(buf).await
    }

    async fn read_n<B: BufMut>(&mut self, buf: B, n: usize) -> io::Result<B> {
        (**self).read_n(buf, n).await
    }

    fn is_read_vectored(&self) -> bool {
        (**self).is_read_vectored()
    }

    async fn read_vectored<B: BufMutSlice<N>, const N: usize>(&mut self, bufs: B) -> io::Result<B> {
        (**self).read_vectored(bufs).await
    }

    async fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
        n: usize,
    ) -> io::Result<B> {
        (**self).read_n_vectored(bufs, n).await
    }
}

/// Read is implemented for `&[u8]` by copying from the slice.
///
/// Note that reading updates the slice to point to the yet unread part. The
/// slice will be empty when EOF is reached.
impl Read for &[u8] {
    async fn read<B: BufMut>(&mut self, mut buf: B) -> io::Result<B> {
        let read = buf.extend_from_slice(self);
        *self = &self[read..];
        Ok(buf)
    }

    async fn read_n<B: BufMut>(&mut self, mut buf: B, n: usize) -> io::Result<B> {
        debug_assert!(
            buf.spare_capacity() >= n,
            "called `[u8]::read_n` with a buffer smaller than `n`",
        );
        if self.len() <= n {
            Err(io::ErrorKind::UnexpectedEof.into())
        } else {
            let read = buf.extend_from_slice(self);
            *self = &self[read..];
            Ok(buf)
        }
    }

    fn is_read_vectored(&self) -> bool {
        true
    }

    async fn read_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        mut bufs: B,
    ) -> io::Result<B> {
        let read = bufs.extend_from_slice(self);
        *self = &self[read..];
        Ok(bufs)
    }

    async fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        mut bufs: B,
        n: usize,
    ) -> io::Result<B> {
        debug_assert!(
            bufs.total_spare_capacity() >= n,
            "called `[u8]::read_n_vectored` with buffers smaller than `n`",
        );
        if self.len() <= n {
            Err(io::ErrorKind::UnexpectedEof.into())
        } else {
            let read = bufs.extend_from_slice(self);
            *self = &self[read..];
            Ok(bufs)
        }
    }
}

impl Read for Vec<u8> {
    async fn read<B: BufMut>(&mut self, mut buf: B) -> io::Result<B> {
        let read = buf.extend_from_slice(self);
        drop(self.drain(..read));
        Ok(buf)
    }

    async fn read_n<B: BufMut>(&mut self, mut buf: B, n: usize) -> io::Result<B> {
        debug_assert!(
            buf.spare_capacity() >= n,
            "called `[u8]::read_n` with a buffer smaller than `n`",
        );
        if self.len() <= n {
            Err(io::ErrorKind::UnexpectedEof.into())
        } else {
            let read = buf.extend_from_slice(self);
            drop(self.drain(..read));
            Ok(buf)
        }
    }

    fn is_read_vectored(&self) -> bool {
        false
    }

    async fn read_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        mut bufs: B,
    ) -> io::Result<B> {
        let read = bufs.extend_from_slice(self);
        drop(self.drain(..read));
        Ok(bufs)
    }

    async fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        mut bufs: B,
        n: usize,
    ) -> io::Result<B> {
        debug_assert!(
            bufs.total_spare_capacity() >= n,
            "called `[u8]::read_n_vectored` with buffers smaller than `n`",
        );
        if self.len() <= n {
            Err(io::ErrorKind::UnexpectedEof.into())
        } else {
            let read = bufs.extend_from_slice(self);
            drop(self.drain(..read));
            Ok(bufs)
        }
    }
}

impl Read for Empty {
    async fn read<B: BufMut>(&mut self, buf: B) -> io::Result<B> {
        Ok(buf)
    }

    async fn read_n<B: BufMut>(&mut self, _: B, _: usize) -> io::Result<B> {
        Err(io::ErrorKind::UnexpectedEof.into())
    }

    fn is_read_vectored(&self) -> bool {
        true
    }

    async fn read_vectored<B: BufMutSlice<N>, const N: usize>(&mut self, bufs: B) -> io::Result<B> {
        Ok(bufs)
    }

    async fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        _: B,
        _: usize,
    ) -> io::Result<B> {
        Err(io::ErrorKind::UnexpectedEof.into())
    }
}

/// Asynchronously writing bytes to a source.
pub trait Write {
    /// Write the bytes in `buf`.
    ///
    /// Returns the number of bytes written. This may we fewer than the length
    /// of `buf`. To ensure that all bytes are written use [`Write::write_all`].
    fn write<B: Buf>(&mut self, buf: B) -> impl Future<Output = io::Result<(B, usize)>>;

    /// Write all bytes in `buf`.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    ///
    /// [`io::ErrorKind::WriteZero`]: std::io::ErrorKind::WriteZero
    fn write_all<B: Buf>(&mut self, buf: B) -> impl Future<Output = io::Result<B>>;

    /// Determines if this `Write`r has an efficient [`write_vectored`]
    /// implementation.
    ///
    /// If a `Write`r does not override the default [`write_vectored`]
    /// implementation, code using it may want to avoid the method all together
    /// and coalesce writes into a single buffer for higher performance.
    ///
    /// The default implementation returns `false`.
    ///
    /// [`write_vectored`]: Write::write_vectored
    fn is_write_vectored(&self) -> bool {
        false
    }

    /// Write the bytes in `bufs`.
    ///
    /// Return the number of bytes written. This may we fewer than the length of
    /// `bufs`. To ensure that all bytes are written use
    /// [`Write::write_vectored_all`].
    fn write_vectored<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> impl Future<Output = io::Result<(B, usize)>>;

    /// Write all bytes in `bufs`.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    ///
    /// [`io::ErrorKind::WriteZero`]: std::io::ErrorKind::WriteZero
    fn write_vectored_all<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> impl Future<Output = io::Result<B>>;
}

impl<T: Write> Write for &mut T {
    async fn write<B: Buf>(&mut self, buf: B) -> io::Result<(B, usize)> {
        (**self).write(buf).await
    }

    async fn write_all<B: Buf>(&mut self, buf: B) -> io::Result<B> {
        (**self).write_all(buf).await
    }

    fn is_write_vectored(&self) -> bool {
        (**self).is_write_vectored()
    }

    async fn write_vectored<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        (**self).write_vectored(bufs).await
    }

    async fn write_vectored_all<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<B> {
        (**self).write_vectored_all(bufs).await
    }
}

impl<T: Write> Write for Box<T> {
    async fn write<B: Buf>(&mut self, buf: B) -> io::Result<(B, usize)> {
        (**self).write(buf).await
    }

    async fn write_all<B: Buf>(&mut self, buf: B) -> io::Result<B> {
        (**self).write_all(buf).await
    }

    fn is_write_vectored(&self) -> bool {
        (**self).is_write_vectored()
    }

    async fn write_vectored<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        (**self).write_vectored(bufs).await
    }

    async fn write_vectored_all<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<B> {
        (**self).write_vectored_all(bufs).await
    }
}

/// Write is implemented for `&mut [u8]` by copying into the slice, overwriting
/// its data.
///
/// Note that writing updates the slice to point to the yet unwritten part. The
/// slice will be empty when it has been completely overwritten.
///
/// If the number of bytes to be written exceeds the size of the slice, write
/// operations will return short writes: ultimately, `Ok(0)`.
impl Write for &mut [u8] {
    async fn write<B: Buf>(&mut self, buf: B) -> io::Result<(B, usize)> {
        let (ptr, buf_len) = unsafe { buf.parts() };
        let written = min(self.len(), buf_len);
        let (this, to_write) = mem::take(self).split_at_mut(written);
        let data = unsafe { slice::from_raw_parts(ptr, written) };
        to_write.copy_from_slice(data);
        *self = this;
        Ok((buf, written))
    }

    async fn write_all<B: Buf>(&mut self, buf: B) -> io::Result<B> {
        match self.write(buf).await {
            Ok((_, 0)) => Err(io::ErrorKind::WriteZero.into()),
            Ok((buf, _)) => Ok(buf),
            Err(err) => Err(err),
        }
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    async fn write_vectored<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        let mut written = 0;
        for buf in bufs.as_io_slices() {
            let max = min(self.len(), buf.len());
            let (this, to_write) = mem::take(self).split_at_mut(max);
            to_write.copy_from_slice(&buf[..max]);
            *self = this;
            written += max;
        }
        Ok((bufs, written))
    }

    async fn write_vectored_all<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<B> {
        match self.write_vectored(bufs).await {
            Ok((_, 0)) => Err(io::ErrorKind::WriteZero.into()),
            Ok((bufs, _)) => Ok(bufs),
            Err(err) => Err(err),
        }
    }
}

impl Write for Vec<u8> {
    async fn write<B: Buf>(&mut self, buf: B) -> io::Result<(B, usize)> {
        self.extend_from_slice(buf.as_slice());
        let written = buf.len();
        Ok((buf, written))
    }

    async fn write_all<B: Buf>(&mut self, buf: B) -> io::Result<B> {
        self.extend_from_slice(buf.as_slice());
        Ok(buf)
    }

    fn is_write_vectored(&self) -> bool {
        false
    }

    async fn write_vectored<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        let mut written = 0;
        for buf in bufs.as_io_slices() {
            self.extend_from_slice(&buf);
            written += buf.len();
        }
        Ok((bufs, written))
    }

    async fn write_vectored_all<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<B> {
        for buf in bufs.as_io_slices() {
            self.extend_from_slice(&buf);
        }
        Ok(bufs)
    }
}

impl Write for Sink {
    async fn write<B: Buf>(&mut self, buf: B) -> io::Result<(B, usize)> {
        let len = buf.len();
        Ok((buf, len))
    }

    async fn write_all<B: Buf>(&mut self, buf: B) -> io::Result<B> {
        Ok(buf)
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    async fn write_vectored<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        let len = bufs.total_len();
        Ok((bufs, len))
    }

    async fn write_vectored_all<B: BufSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
    ) -> io::Result<B> {
        Ok(bufs)
    }
}
