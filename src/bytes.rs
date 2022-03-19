//! Traits to work with bytes.
//!
//! This module contains two traits:
//!  * [`Bytes`] to work a single buffer, e.g. `Vec<u8>`, and
//!  * [`BytesVectored`] to work with multiple buffers and using vectored I/O,
//!    e.g. `Vec<Vec<u8>>`.
//!
//! The basic design of both traits is the same and is fairly simple. It's split
//! into two methods. Usage starts with a call to [`as_bytes`]/[`as_bufs`],
//! which returns a slice to unitialised bytes. The caller should then fill that
//! slice with valid bytes, e.g. by receiving bytes from a socket. Once the
//! slice is (partially) filled the caller should call
//! [`update_length`]/[`update_lengths`], which updates the length of the
//! buffer.
//!
//! [`as_bytes`]: Bytes::as_bytes
//! [`as_bufs`]: BytesVectored::as_bufs
//! [`update_length`]: Bytes::update_length
//! [`update_lengths`]: BytesVectored::update_lengths

use std::cmp::min;
use std::io::IoSliceMut;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::{fmt, slice};

/// Trait to make easier to work with uninitialised buffers.
///
/// This is implemented for common types such as `Vec<u8>`, [see below].
///
/// [see below]: #foreign-impls
pub trait Bytes {
    /// Returns itself as a slice of bytes that may or may not be initialised.
    ///
    /// # Notes
    ///
    /// The implementation must guarantee that two calls (without a call to
    /// [`update_length`] in between) returns the same slice of bytes.
    ///
    /// [`update_length`]: Bytes::update_length
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>];

    /// Returns the length of the buffer as returned by [`as_bytes`].
    ///
    /// [`as_bytes`]: Bytes::as_bytes
    fn spare_capacity(&self) -> usize;

    /// Returns `true` if the buffer has spare capacity.
    fn has_spare_capacity(&self) -> bool {
        self.spare_capacity() == 0
    }

    /// Update the length of the byte slice, marking `n` bytes as initialised.
    ///
    /// # Safety
    ///
    /// The caller must ensure that at least the first `n` bytes returned by
    /// [`as_bytes`] are initialised.
    ///
    /// [`as_bytes`]: Bytes::as_bytes
    ///
    /// # Notes
    ///
    /// If this method is not implemented correctly methods such as
    /// [`TcpStream::recv_n`] will not work correctly (as the buffer will
    /// overwrite itself on successive reads).
    ///
    /// [`TcpStream::recv_n`]: crate::net::TcpStream::recv_n
    unsafe fn update_length(&mut self, n: usize);

    /// Wrap the buffer in `LimitedBytes`, which limits the amount of bytes used
    /// to `limit`.
    ///
    /// [`LimitedBytes::into_inner`] can be used to retrieve the buffer again,
    /// or a mutable reference to the buffer can be used and the limited buffer
    /// be dropped after usage.
    fn limit(self, limit: usize) -> LimitedBytes<Self>
    where
        Self: Sized,
    {
        LimitedBytes { buf: self, limit }
    }
}

impl<B> Bytes for &mut B
where
    B: Bytes + ?Sized,
{
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        (&mut **self).as_bytes()
    }

    fn spare_capacity(&self) -> usize {
        (&**self).spare_capacity()
    }

    fn has_spare_capacity(&self) -> bool {
        (&**self).has_spare_capacity()
    }

    unsafe fn update_length(&mut self, n: usize) {
        (&mut **self).update_length(n)
    }
}

/// The implementation for `Vec<u8>` only uses the uninitialised capacity of the
/// vector. In other words the bytes currently in the vector remain untouched.
///
/// # Examples
///
/// The following example shows that the bytes already in the vector remain
/// untouched.
///
/// ```
/// use heph::bytes::Bytes;
///
/// let mut buf = Vec::with_capacity(100);
/// buf.extend(b"Hello world!");
///
/// write_bytes(b" Hello mars!", &mut buf);
///
/// assert_eq!(&*buf, b"Hello world! Hello mars!");
///
/// fn write_bytes<B>(src: &[u8], mut buf: B) where B: Bytes {
///     // Writes `src` to `buf`.
/// #   let dst = buf.as_bytes();
/// #   let len = std::cmp::min(src.len(), dst.len());
/// #   // Safety: both the src and dst pointers are good. And we've ensured
/// #   // that the length is correct, not overwriting data we don't own or
/// #   // reading data we don't own.
/// #   unsafe {
/// #       std::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr().cast(), len);
/// #       buf.update_length(len);
/// #   }
/// }
/// ```
impl Bytes for Vec<u8> {
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        self.spare_capacity_mut()
    }

    fn spare_capacity(&self) -> usize {
        self.capacity() - self.len()
    }

    fn has_spare_capacity(&self) -> bool {
        self.capacity() != self.len()
    }

    unsafe fn update_length(&mut self, n: usize) {
        let new = self.len() + n;
        debug_assert!(self.capacity() >= new);
        self.set_len(new);
    }
}

/// A version of [`IoSliceMut`] that allows the buffer to be uninitialised.
///
/// [`IoSliceMut`]: std::io::IoSliceMut
#[repr(transparent)]
pub struct MaybeUninitSlice<'a>(socket2::MaybeUninitSlice<'a>);

impl<'a> fmt::Debug for MaybeUninitSlice<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a> MaybeUninitSlice<'a> {
    /// Creates a new `MaybeUninitSlice` wrapping a byte slice.
    ///
    /// # Panics
    ///
    /// Panics on Windows if the slice is larger than 4GB.
    pub fn new(buf: &'a mut [MaybeUninit<u8>]) -> MaybeUninitSlice<'a> {
        MaybeUninitSlice(socket2::MaybeUninitSlice::new(buf))
    }

    /// Creates a new `MaybeUninitSlice` from a [`Vec`]tor.
    ///
    /// Similar to the [`Bytes`] implementation for `Vec<u8>` this only uses the
    /// uninitialised capacity of the vector.
    ///
    /// # Panics
    ///
    /// Panics on Windows if the vector's uninitialised capacity is larger than
    /// 4GB.
    pub fn from_vec(buf: &'a mut Vec<u8>) -> MaybeUninitSlice<'a> {
        MaybeUninitSlice(socket2::MaybeUninitSlice::new(buf.as_bytes()))
    }

    fn limit(&mut self, limit: usize) {
        let len = self.len();
        assert!(len >= limit);
        self.0 = unsafe {
            // SAFETY: this should be the line below, but I couldn't figure out
            // the lifetime. Since we're only making the slices smaller (as
            // checked by the assert above) this should be safe.
            //self.0 = socket2::MaybeUninitSlice::new(&mut self[..limit]);
            socket2::MaybeUninitSlice::new(slice::from_raw_parts_mut(self.0.as_mut_ptr(), limit))
        };
    }

    /// Returns `bufs` as [`socket2::MaybeUninitSlice`].
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn as_socket2<'b>(
        bufs: &'b mut [MaybeUninitSlice<'a>],
    ) -> &'b mut [socket2::MaybeUninitSlice<'a>] {
        // Safety: this is safe because `MaybeUninitSlice` has the
        // `repr(transparent)` attribute.
        unsafe { &mut *(bufs as *mut _ as *mut _) }
    }

    /// Returns `bufs` as [`IoSliceMut`].
    ///
    /// # Unsafety
    ///
    /// This is unsound.
    ///
    /// Reading from the returned slice is UB.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) unsafe fn as_io<'b>(
        bufs: &'b mut [MaybeUninitSlice<'a>],
    ) -> &'b mut [IoSliceMut<'a>] {
        &mut *(bufs as *mut _ as *mut _)
    }
}

impl<'a> Deref for MaybeUninitSlice<'a> {
    type Target = [MaybeUninit<u8>];

    fn deref(&self) -> &[MaybeUninit<u8>] {
        &*self.0
    }
}

impl<'a> DerefMut for MaybeUninitSlice<'a> {
    fn deref_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        &mut *self.0
    }
}

/// Trait to make easier to work with uninitialised buffers using vectored I/O.
///
/// This trait is implemented for arrays and tuples. When all of buffers are
/// *homogeneous*, i.e. of the same type, the array implementation is the
/// easiest to use along side with the [`Bytes`] trait. If however the buffers
/// are *heterogeneous*, i.e. of different types, the tuple implementation can
/// be used. See the examples below.
///
/// # Examples
///
/// Using the homogeneous array implementation.
///
/// ```
/// # #![feature(maybe_uninit_write_slice)]
/// use heph::bytes::BytesVectored;
///
/// let mut buf1 = Vec::with_capacity(12);
/// let mut buf2 = Vec::with_capacity(1);
/// let mut buf3 = Vec::with_capacity(5);
/// let mut buf4 = Vec::with_capacity(10); // Has extra capacity.
///
/// let bufs = [&mut buf1, &mut buf2, &mut buf3, &mut buf4];
/// let text = b"Hello world. From mars!";
/// let bytes_written = write_vectored(bufs, text);
/// assert_eq!(bytes_written, text.len());
///
/// assert_eq!(buf1, b"Hello world.");
/// assert_eq!(buf2, b" ");
/// assert_eq!(buf3, b"From ");
/// assert_eq!(buf4, b"mars!");
///
/// /// Writes `text` to the `bufs`.
/// fn write_vectored<B>(mut bufs: B, text: &[u8]) -> usize
///     where B: BytesVectored,
/// {
///     // Implementation is not relevant to the example.
/// #   let mut written = 0;
/// #   let mut left = text;
/// #   for buf in bufs.as_bufs().as_mut().iter_mut() {
/// #       let n = std::cmp::min(buf.len(), left.len());
/// #       let _ = std::mem::MaybeUninit::write_slice(&mut buf[..n], &left[..n]);
/// #       left = &left[n..];
/// #       written += n;
/// #       if left.is_empty() {
/// #           break;
/// #       }
/// #   }
/// #   // NOTE: we could update the length of the buffers in the loop above,
/// #   // but this also acts as a smoke test for the implementation and this is
/// #   // what would happen with actual vectored I/O.
/// #   unsafe { bufs.update_lengths(written); }
/// #   written
/// }
/// ```
///
/// Using the heterogeneous tuple implementation.
///
/// ```
/// # #![feature(maybe_uninit_uninit_array, maybe_uninit_slice, maybe_uninit_write_slice)]
/// use std::mem::MaybeUninit;
///
/// use heph::bytes::{Bytes, BytesVectored};
///
/// // Buffers of different types.
/// let mut buf1 = Vec::with_capacity(12);
/// let mut buf2 = StackBuf::new(); // Has extra capacity.
///
/// // Using tuples we can use different kind of buffers. Here we use a `Vec` and
/// // our own `StackBuf` type.
/// let bufs = (&mut buf1, &mut buf2);
/// let text = b"Hello world. From mars!";
/// let bytes_written = write_vectored(bufs, text);
/// assert_eq!(bytes_written, text.len());
///
/// assert_eq!(buf1, b"Hello world.");
/// assert_eq!(buf2.bytes(), b" From mars!");
///
/// /// Writes `text` to the `bufs`.
/// fn write_vectored<B>(mut bufs: B, text: &[u8]) -> usize
///     where B: BytesVectored,
/// {
///     // Implementation is not relevant to the example.
/// #   let mut written = 0;
/// #   let mut left = text;
/// #   for buf in bufs.as_bufs().as_mut().iter_mut() {
/// #       let n = std::cmp::min(buf.len(), left.len());
/// #       let _ = MaybeUninit::write_slice(&mut buf[..n], &left[..n]);
/// #       left = &left[n..];
/// #       written += n;
/// #       if left.is_empty() {
/// #           break;
/// #       }
/// #   }
/// #   // NOTE: we could update the length of the buffers in the loop above,
/// #   // but this also acts as a smoke test for the implementation and this is
/// #   // what would happen with actual vectored I/O.
/// #   unsafe { bufs.update_lengths(written); }
/// #   written
/// }
///
/// /// Custom stack buffer type that implements the `Bytes` trait.
/// struct StackBuf {
///     bytes: [MaybeUninit<u8>; 4096],
///     initialised: usize,
/// }
///
/// impl StackBuf {
///     fn new() -> StackBuf {
///         StackBuf {
///             bytes: MaybeUninit::uninit_array(),
///             initialised: 0,
///         }
///     }
///
///     fn bytes(&self) -> &[u8] {
///         unsafe { MaybeUninit::slice_assume_init_ref(&self.bytes[..self.initialised]) }
///     }
/// }
///
/// impl Bytes for StackBuf {
///     fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
///         &mut self.bytes[self.initialised..]
///     }
///
///     fn spare_capacity(&self) -> usize {
///         self.bytes.len() - self.initialised
///     }
///
///     fn has_spare_capacity(&self) -> bool {
///         self.bytes.len() != self.initialised
///     }
///
///     unsafe fn update_length(&mut self, n: usize) {
///         self.initialised += n;
///     }
/// }
/// ```
pub trait BytesVectored {
    /// Type used as slice of buffers, usually this is an array.
    type Bufs<'b>: AsMut<[MaybeUninitSlice<'b>]>
    where
        Self: 'b;

    /// Returns itself as a slice of [`MaybeUninitSlice`].
    fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b>;

    /// Returns the total length of the buffers as returned by [`as_bufs`].
    ///
    /// [`as_bufs`]: BytesVectored::as_bufs
    fn spare_capacity(&self) -> usize;

    /// Returns `true` if (one of) the buffers has spare capacity.
    fn has_spare_capacity(&self) -> bool {
        self.spare_capacity() == 0
    }

    /// Update the length of the buffers in the slice.
    ///
    /// # Safety
    ///
    /// The caller must ensure that at least the first `n` bytes returned by
    /// [`as_bufs`] are initialised, starting at the first buffer continuing
    /// into the next one, etc.
    ///
    /// [`as_bufs`]: BytesVectored::as_bufs
    ///
    /// # Notes
    ///
    /// If this method is not implemented correctly methods such as
    /// [`TcpStream::recv_n_vectored`] will not work correctly (as the buffer
    /// will overwrite itself on successive reads).
    ///
    /// [`TcpStream::recv_n_vectored`]: crate::net::TcpStream::recv_n_vectored
    unsafe fn update_lengths(&mut self, n: usize);

    /// Wrap the buffer in `LimitedBytes`, which limits the amount of bytes used
    /// to `limit`.
    ///
    /// [`LimitedBytes::into_inner`] can be used to retrieve the buffer again,
    /// or a mutable reference to the buffer can be used and the limited buffer
    /// be dropped after usage.
    fn limit(self, limit: usize) -> LimitedBytes<Self>
    where
        Self: Sized,
    {
        LimitedBytes { buf: self, limit }
    }
}

impl<B> BytesVectored for &mut B
where
    B: BytesVectored + ?Sized,
{
    type Bufs<'b> = B::Bufs<'b> where Self: 'b;

    fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b> {
        (&mut **self).as_bufs()
    }

    fn spare_capacity(&self) -> usize {
        (&**self).spare_capacity()
    }

    fn has_spare_capacity(&self) -> bool {
        (&**self).has_spare_capacity()
    }

    unsafe fn update_lengths(&mut self, n: usize) {
        (&mut **self).update_lengths(n)
    }
}

impl<B, const N: usize> BytesVectored for [B; N]
where
    B: Bytes,
{
    type Bufs<'b> = [MaybeUninitSlice<'b>; N] where Self: 'b;

    fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b> {
        let mut bufs = MaybeUninit::uninit_array::<N>();
        for (i, buf) in self.iter_mut().enumerate() {
            let _ = bufs[i].write(MaybeUninitSlice::new(buf.as_bytes()));
        }
        // Safety: initialised the buffers above.
        unsafe { MaybeUninit::array_assume_init(bufs) }
    }

    fn spare_capacity(&self) -> usize {
        self.iter().map(Bytes::spare_capacity).sum()
    }

    fn has_spare_capacity(&self) -> bool {
        self.iter().any(Bytes::has_spare_capacity)
    }

    unsafe fn update_lengths(&mut self, n: usize) {
        let mut left = n;
        for buf in self.iter_mut() {
            let n = min(left, buf.spare_capacity());
            buf.update_length(n);
            left -= n;
            if left == 0 {
                return;
            }
        }
    }
}

macro_rules! impl_vectored_bytes_tuple {
    ( $N: tt : $( $t: ident $idx: tt ),+ ) => {
        impl<$( $t ),+> BytesVectored for ( $( $t ),+ )
            where $( $t: Bytes ),+
        {
            type Bufs<'b> = [MaybeUninitSlice<'b>; $N] where Self: 'b;

            fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b> {
                let mut bufs = MaybeUninit::uninit_array::<$N>();
                $(
                    let _ = bufs[$idx].write(MaybeUninitSlice::new(self.$idx.as_bytes()));
                )+
                // Safety: initialised the buffers above.
                unsafe { MaybeUninit::array_assume_init(bufs) }
            }

            fn spare_capacity(&self) -> usize {
                $( self.$idx.spare_capacity() + )+ 0
            }

            fn has_spare_capacity(&self) -> bool {
                $( self.$idx.has_spare_capacity() || )+ false
            }

            unsafe fn update_lengths(&mut self, n: usize) {
                let mut left = n;
                $(
                    let n = min(left, self.$idx.spare_capacity());
                    self.$idx.update_length(n);
                    left -= n;
                    if left == 0 {
                        return;
                    }
                )+
            }
        }
    };
}

impl_vectored_bytes_tuple! { 12: B0 0, B1 1, B2 2, B3 3, B4 4, B5 5, B6 6, B7 7, B8 8, B9 9, B10 10, B11 11 }
impl_vectored_bytes_tuple! { 11: B0 0, B1 1, B2 2, B3 3, B4 4, B5 5, B6 6, B7 7, B8 8, B9 9, B10 10 }
impl_vectored_bytes_tuple! { 10: B0 0, B1 1, B2 2, B3 3, B4 4, B5 5, B6 6, B7 7, B8 8, B9 9 }
impl_vectored_bytes_tuple! { 9: B0 0, B1 1, B2 2, B3 3, B4 4, B5 5, B6 6, B7 7, B8 8 }
impl_vectored_bytes_tuple! { 8: B0 0, B1 1, B2 2, B3 3, B4 4, B5 5, B6 6, B7 7 }
impl_vectored_bytes_tuple! { 7: B0 0, B1 1, B2 2, B3 3, B4 4, B5 5, B6 6 }
impl_vectored_bytes_tuple! { 6: B0 0, B1 1, B2 2, B3 3, B4 4, B5 5 }
impl_vectored_bytes_tuple! { 5: B0 0, B1 1, B2 2, B3 3, B4 4 }
impl_vectored_bytes_tuple! { 4: B0 0, B1 1, B2 2, B3 3 }
impl_vectored_bytes_tuple! { 3: B0 0, B1 1, B2 2 }
impl_vectored_bytes_tuple! { 2: B0 0, B1 1 }

/// Wrapper to limit the number of bytes `B` can use.
///
/// See [`Bytes::limit`] and [`BytesVectored::limit`].
#[derive(Debug)]
pub struct LimitedBytes<B> {
    buf: B,
    limit: usize,
}

impl<B> LimitedBytes<B> {
    /// Returns the underlying buffer.
    pub fn into_inner(self) -> B {
        self.buf
    }
}

impl<B> Bytes for LimitedBytes<B>
where
    B: Bytes,
{
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        let bytes = self.buf.as_bytes();
        if bytes.len() > self.limit {
            &mut bytes[..self.limit]
        } else {
            bytes
        }
    }

    fn spare_capacity(&self) -> usize {
        min(self.buf.spare_capacity(), self.limit)
    }

    fn has_spare_capacity(&self) -> bool {
        self.spare_capacity() > 0
    }

    unsafe fn update_length(&mut self, n: usize) {
        self.buf.update_length(n);
        self.limit -= n;
    }
}

impl<B> BytesVectored for LimitedBytes<B>
where
    B: BytesVectored,
{
    type Bufs<'b> = B::Bufs<'b> where Self: 'b;

    fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b> {
        let mut bufs = self.buf.as_bufs();
        let mut left = self.limit;
        let mut iter = bufs.as_mut().iter_mut();
        while let Some(buf) = iter.next() {
            let len = buf.len();
            if left > len {
                left -= len;
            } else {
                buf.limit(left);
                for buf in iter {
                    *buf = MaybeUninitSlice::new(&mut []);
                }
                break;
            }
        }
        bufs
    }

    fn spare_capacity(&self) -> usize {
        if self.limit == 0 {
            0
        } else {
            min(self.buf.spare_capacity(), self.limit)
        }
    }

    fn has_spare_capacity(&self) -> bool {
        self.limit != 0 && self.buf.has_spare_capacity()
    }

    unsafe fn update_lengths(&mut self, n: usize) {
        self.buf.update_lengths(n);
        self.limit -= n;
    }
}
