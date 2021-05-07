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
//!
//! # I/O with Heph's socket
//!
//! The different socket types provide two or three variants of most I/O
//! functions. The `try_*` funtions, which makes the system calls once. For
//! example [`TcpStream::try_send`] calls `send(2)` once, not handling any
//! errors (including [`WouldBlock`] errors!).
//!
//! In addition they provide a [`Future`] function which handles would block
//! errors. For `TcpStream::try_send` the future version is [`TcpStream::send`],
//! i.e. without the `try_` prefix.
//!
//! Finally for a lot of function a convenience version is provided that handle
//! various cases. For example with sending you might want to ensure all bytes
//! are send, for this you can use [`TcpStream::send_all`]. But also see
//! functions such as [`TcpStream::recv_n`]; which receives at least `n` bytes,
//! or [`TcpStream::send_entire_file`]; which sends an entire file using the
//! `sendfile(2)` system call.
//!
//! [`WouldBlock`]: io::ErrorKind::WouldBlock
//! [`Future`]: std::future::Future
//!
//! # Notes
//!
//! All types in the `net` module around [bound] to an actor. See the
//! [`actor::Bound`] trait for more information.
//!
//! [bound]: crate::actor::Bound
//! [`actor::Bound`]: crate::actor::Bound

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
    unsafe fn update_length(&mut self, n: usize);
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
/// use heph::net::{Bytes, BytesVectored};
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
    type Bufs<'b>: AsMut<[MaybeUninitSlice<'b>]>;

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
    type Bufs<'b> = [MaybeUninitSlice<'b>; N];

    fn as_bufs<'b>(&'b mut self) -> Self::Bufs<'b> {
        let mut bufs = MaybeUninit::uninit_array::<N>();
        for (i, buf) in self.iter_mut().enumerate() {
            let _ = bufs[i].write(MaybeUninitSlice::new(buf.as_bytes()));
        }
        // Safety: initialised the buffers above.
        unsafe { MaybeUninit::array_assume_init(bufs) }
    }

    fn spare_capacity(&self) -> usize {
        self.iter().map(|b| b.spare_capacity()).sum()
    }

    fn has_spare_capacity(&self) -> bool {
        self.iter().any(|b| b.has_spare_capacity())
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
            type Bufs<'b> = [MaybeUninitSlice<'b>; $N];

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
