use std::io;

use crate::io::{BufMut, BufMutSlice};

/// Asynchronous reading bytes from a source.
pub trait Read {
    /// Read bytes, writing them into `buf`.
    ///
    /// # Notes
    ///
    /// The caller must always check if at least one byte was read as reading
    /// zero bytes is an indication that no more bytes can be read. Failing to
    /// do so can result in an infinite loop.
    async fn read<B: BufMut>(&mut self, buf: B) -> io::Result<B>;

    /// Read at least `n` bytes, writing them into `buf`.
    ///
    /// This returns [`io::ErrorKind::UnexpectedEof`] if less than `n` bytes
    /// could be read.
    ///
    /// [`io::ErrorKind::UnexpectedEof`]: std::io::ErrorKind::UnexpectedEof
    async fn read_n<B: BufMut>(&mut self, buf: B, n: usize) -> io::Result<B>;

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
    async fn read_vectored<B: BufMutSlice<N>, const N: usize>(&mut self, bufs: B) -> io::Result<B>;

    /// Read at least `n` bytes, writing them into `bufs`.
    async fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &mut self,
        bufs: B,
        n: usize,
    ) -> io::Result<B>;
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
        let read = buf.extend_from_slice(&**self);
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
            let read = buf.extend_from_slice(&**self);
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
        let read = bufs.extend_from_slice(&**self);
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
            let read = bufs.extend_from_slice(&**self);
            *self = &self[read..];
            Ok(bufs)
        }
    }
}
