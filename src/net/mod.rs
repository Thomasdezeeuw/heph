//! Network related types.
//!
//! The network module support two types of protocols:
//!
//! * [Transmission Control Protocol] (TCP) module provides three main types:
//!   * A [TCP stream] between a local and a remote socket.
//!   * A [TCP listening socket], a socket used to listen for connections.
//!   * A [TCP server], listens for connections and starts a new actor for each.
//! * [User Datagram Protocol] (UDP) only provides a single socket type:
//!   * [`UdpSocket`].
//!
//! [Transmission Control Protocol]: crate::net::tcp
//! [TCP stream]: crate::net::TcpStream
//! [TCP listening socket]: crate::net::TcpListener
//! [TCP server]: crate::net::TcpServer
//! [User Datagram Protocol]: crate::net::udp
//! [`UdpSocket`]: crate::net::UdpSocket

use std::cmp::min;
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::{fmt, io};

use socket2::SockAddr;

/// A macro to try an I/O function.
///
/// Note that this is used in the tcp and udp modules and has to be defined
/// before them, otherwise this would have been place below.
macro_rules! try_io {
    ($op: expr) => {
        loop {
            match $op {
                Ok(ok) => break Poll::Ready(Ok(ok)),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    };
}

pub mod tcp;
pub mod udp;

#[doc(no_inline)]
pub use tcp::{TcpListener, TcpServer, TcpStream};
#[doc(no_inline)]
pub use udp::UdpSocket;

/// Trait to make easier to work with slices of (uninitialised) bytes.
///
/// This is implemented for common types such as `&mut[u8]` and `Vec<u8>`, [see
/// below].
///
/// [see below]: #foreign-impls
pub trait Bytes {
    /// Returns itself as a slice of bytes that may or may not be initialised.
    ///
    /// The implementation must guarantee that two calls (without any call to
    /// [`update_length`] in between returns the same slice of bytes.
    ///
    /// [`update_length`]: Bytes::update_length
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>];

    /// Update the length of the byte slice.
    ///
    /// # Safety
    ///
    /// The caller must ensure that at least `n` of the bytes returned by
    /// [`as_bytes`] are initialised.
    ///
    /// [`as_bytes`]: Bytes::as_bytes
    unsafe fn update_length(&mut self, n: usize);
}

impl<B> Bytes for &mut B
where
    B: Bytes + ?Sized,
{
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        (&mut **self).as_bytes()
    }

    unsafe fn update_length(&mut self, n: usize) {
        (&mut **self).update_length(n)
    }
}

impl Bytes for [MaybeUninit<u8>] {
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        self
    }

    unsafe fn update_length(&mut self, _: usize) {
        // Can't update the length of a slice.
    }
}

/* TODO: this implementation is unsound, see issue #308.
impl Bytes for [u8] {
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        // Safety: `MaybeUninit<u8>` is guaranteed to have the same layout as
        // `u8` so it same to cast the pointer.
        unsafe { &mut *(self as *mut [u8] as *mut [MaybeUninit<u8>]) }
    }

    unsafe fn update_length(&mut self, _: usize) {
        // Can't update the length of a slice.
    }
}
*/

impl<const N: usize> Bytes for [MaybeUninit<u8>; N] {
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        self.as_mut_slice().as_bytes()
    }

    unsafe fn update_length(&mut self, _: usize) {
        // Can't update the length of an array.
    }
}

/* TODO: this implementation is unsound, see issue #308.
impl<const N: usize> Bytes for [u8; N] {
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        self.as_mut_slice().as_bytes()
    }

    unsafe fn update_length(&mut self, _: usize) {
        // Can't update the length of an array.
    }
}
*/

/// The implementation for `Vec<u8>` only uses the uninitialised capacity of the
/// vector. In other words the bytes currently in the vector remain untouched.
///
/// # Examples
///
/// The following example shows that the bytes already in the vector remain
/// untouched.
///
/// ```
/// use heph::net::Bytes;
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
    // NOTE: keep this function in sync with the impl below.
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        self.spare_capacity_mut()
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

    /// Returns `bufs` as [`socket2::MaybeUninitSlice`].
    #[allow(clippy::wrong_self_convention)]
    fn as_socket2<'b>(
        bufs: &'b mut [MaybeUninitSlice<'a>],
    ) -> &'b mut [socket2::MaybeUninitSlice<'a>] {
        // Safety: this is safe because `MaybeUninitSlice` has the
        // `repr(transparent)` attribute.
        unsafe { &mut *(bufs as *mut _ as *mut _) }
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

impl<'a> Bytes for MaybeUninitSlice<'a> {
    // NOTE: keep this function in sync with the impl below.
    fn as_bytes(&mut self) -> &mut [MaybeUninit<u8>] {
        self
    }

    unsafe fn update_length(&mut self, _: usize) {
        // Can't update the length of an array.
    }
}

/// Trait to make easier to work with vectored I/O using uninitialised bytes.
///
/// This is implemented for arrays and tuples. When all of buffers are
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
/// use heph::net::BytesVectored;
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
/// #   use heph::net::Bytes;
/// #   let mut written = 0;
/// #   let mut left = text;
/// #   for buf in bufs.as_bufs().as_mut().iter_mut() {
/// #       let dst = buf.as_bytes();
/// #       let n = std::cmp::min(dst.len(), left.len());
/// #       let _ = std::mem::MaybeUninit::write_slice(&mut dst[..n], &left[..n]);
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
/// # #![feature(maybe_uninit_slice, maybe_uninit_write_slice)]
/// use std::mem::MaybeUninit;
///
/// use heph::net::BytesVectored;
///
/// // Buffers of different types.
/// let mut buf1 = Vec::with_capacity(12);
/// let mut buf2 = [MaybeUninit::uninit(); 1];
/// let mut buf3 = &mut [MaybeUninit::uninit(); 20]; // Has extra capacity.
///
/// // Using tuples we can different kind of buffers. Here we use a `Vec`, array
/// // and a slice.
/// let bufs = (&mut buf1, &mut buf2, &mut buf3);
/// let text = b"Hello world. From mars!";
/// let bytes_written = write_vectored(bufs, text);
/// assert_eq!(bytes_written, text.len());
///
/// assert_eq!(buf1, b"Hello world.");
/// assert_eq!(unsafe { MaybeUninit::slice_assume_init_ref(&buf2) }, b" ");
/// assert_eq!(unsafe { MaybeUninit::slice_assume_init_ref(&buf3[..10]) }, b"From mars!");
///
/// /// Writes `text` to the `bufs`.
/// fn write_vectored<B>(mut bufs: B, text: &[u8]) -> usize
///     where B: BytesVectored,
/// {
///     // Implementation is not relevant to the example.
/// #   use heph::net::Bytes;
/// #   let mut written = 0;
/// #   let mut left = text;
/// #   for buf in bufs.as_bufs().as_mut().iter_mut() {
/// #       let dst = buf.as_bytes();
/// #       let n = std::cmp::min(dst.len(), left.len());
/// #       let _ = MaybeUninit::write_slice(&mut dst[..n], &left[..n]);
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
pub trait BytesVectored {
    /// Type used as slice of buffers, usually this is an array.
    type Bufs<'b>: AsMut<[MaybeUninitSlice<'b>]>;

    /// Returns itself as a slice of [`MaybeUninitSlice`].
    fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b>;

    /// Update the length of the buffers in the slice.
    ///
    /// # Safety
    ///
    /// The caller must ensure that at least `n` of the bytes returned by
    /// [`as_bufs`] are initialised, starting at the first buffer.
    ///
    /// [`as_bufs`]: BytesVectored::as_bufs
    unsafe fn update_lengths(&mut self, n: usize);
}

impl<B> BytesVectored for &mut B
where
    B: BytesVectored + ?Sized,
{
    type Bufs<'b> = B::Bufs<'b>;

    fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b> {
        (&mut **self).as_bufs()
    }

    unsafe fn update_lengths(&mut self, n: usize) {
        (&mut **self).update_lengths(n)
    }
}

impl<B, const N: usize> BytesVectored for [B; N]
where
    B: Bytes,
{
    type Bufs<'b> = [MaybeUninitSlice<'b>; N];

    fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b> {
        // Safety: `MaybeUninitSlice` doesn't implement `Drop`, so it's safe to
        // leak it.
        let mut bufs = MaybeUninit::uninit_array::<N>();
        for (i, buf) in self.iter_mut().enumerate() {
            let _ = bufs[i].write(MaybeUninitSlice::new(buf.as_bytes()));
        }
        unsafe { MaybeUninit::array_assume_init(bufs) }
    }

    unsafe fn update_lengths(&mut self, n: usize) {
        let mut left = n;
        for buf in self.iter_mut() {
            let n = min(left, buf.as_bytes().len());
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
            type Bufs<'b> = [MaybeUninitSlice<'b>; $N];

            fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b> {
                // Safety: `MaybeUninitSlice` doesn't implement `Drop`, so it's
                // safe to leak it.
                let mut bufs = MaybeUninit::uninit_array::<$N>();
                $(
                    let _ = bufs[$idx].write(MaybeUninitSlice::new(self.$idx.as_bytes()));
                )+
                unsafe { MaybeUninit::array_assume_init(bufs) }
            }

            unsafe fn update_lengths(&mut self, n: usize) {
                let mut left = n;
                $(
                    let n = min(left, self.$idx.as_bytes().len());
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

/// Convert a `socket2:::SockAddr` into a `std::net::SocketAddr`.
#[allow(clippy::needless_pass_by_value)]
fn convert_address(address: SockAddr) -> io::Result<SocketAddr> {
    match address.as_socket() {
        Some(address) => Ok(address),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid address family (not IPv4 or IPv6)",
        )),
    }
}
