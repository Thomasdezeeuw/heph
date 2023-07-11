//! Buffers.

use std::borrow::Cow;
use std::cmp::min;
use std::io::IoSlice;
use std::mem::MaybeUninit;
use std::slice;
use std::sync::Arc;

/// Trait that defines the behaviour of buffers used in reading, which requires
/// mutable access.
///
/// This is implemented for common types such as `Vec<u8>`, [see below].
///
/// [see below]: #foreign-impls
///
/// # Safety
///
/// Unlike normal buffers the buffer implementations for Heph have additional
/// requirements because Heph uses io_uring.
///
/// If the operation (that uses this buffer) is not polled to completion, i.e.
/// the `Future` is dropped before it returns `Poll::Ready`, the kernel still
/// has access to the buffer and will still attempt to write into it. This means
/// that we must delay deallocation in such a way that the kernel will not write
/// into memory we don't have access to any more. This makes, for example, stack
/// based buffers unfit to implement `BufMut`. Because we can't delay the
/// deallocation once its dropped and the kernel will overwrite part of your
/// stack (where the buffer used to be)!
pub unsafe trait BufMut: 'static {
    /// Returns the writable buffer as pointer and length parts.
    ///
    /// # Safety
    ///
    /// Only initialised bytes may be written to the pointer returned. The
    /// pointer *may* point to uninitialised bytes, so reading from the pointer
    /// is UB.
    ///
    /// The implementation must ensure that the pointer is valid, i.e. not null
    /// and pointing to memory owned by the buffer. Furthermore it must ensure
    /// that the returned length is, in combination with the pointer, valid. In
    /// other words the memory the pointer and length are pointing to must be a
    /// valid memory address and owned by the buffer.
    ///
    /// # Why not a slice?
    ///
    /// Returning a slice `&[u8]` would prevent us to use unitialised bytes,
    /// meaning we have to zero the buffer before usage, not ideal for
    /// performance. So, naturally you would suggest `&[MaybeUninit<u8>]`,
    /// however that would prevent buffer types with only initialised bytes.
    /// Returning a slice with `MaybeUninit` to such as type would be unsound as
    /// it would allow the caller to write unitialised bytes without using
    /// `unsafe`.
    unsafe fn parts_mut(&mut self) -> (*mut u8, usize);

    /// Update the length of the byte slice, marking `n` bytes as initialised.
    ///
    /// # Safety
    ///
    /// The caller must ensure that at least the first `n` bytes returned by
    /// [`parts_mut`] are initialised.
    ///
    /// [`parts_mut`]: BufMut::parts_mut
    ///
    /// # Notes
    ///
    /// If this method is not implemented correctly methods such as
    /// [`TcpStream::recv_n`] will not work correctly (as the buffer will
    /// overwrite itself on successive reads).
    ///
    /// [`TcpStream::recv_n`]: crate::net::TcpStream::recv_n
    unsafe fn update_length(&mut self, n: usize);

    /// Extend the buffer with `bytes`, returns the number of bytes copied.
    fn extend_from_slice(&mut self, bytes: &[u8]) -> usize {
        let (ptr, capacity) = unsafe { self.parts_mut() };
        // SAFETY: `parts_mut` requirements are the same for `copy_bytes`.
        let written = unsafe { copy_bytes(ptr, capacity, bytes) };
        // SAFETY: just written the bytes in the call above.
        unsafe { self.update_length(written) };
        written
    }

    /// Returns the length of the buffer as returned by [`parts_mut`].
    ///
    /// [`parts_mut`]: BufMut::parts_mut
    fn spare_capacity(&self) -> usize;

    /// Returns `true` if the buffer has spare capacity.
    fn has_spare_capacity(&self) -> bool {
        self.spare_capacity() != 0
    }

    /// Wrap the buffer in `Limited`, which limits the amount of bytes used to
    /// `limit`.
    ///
    /// [`Limited::into_inner`] can be used to retrieve the buffer again,
    /// or a mutable reference to the buffer can be used and the limited buffer
    /// be dropped after usage.
    fn limit(self, limit: usize) -> Limited<Self>
    where
        Self: Sized,
    {
        Limited { buf: self, limit }
    }
}

/// Copies bytes from `src` to `dst`, copies up to `min(dst_len, src.len())`,
/// i.e. it won't write beyond `dst` or read beyond `src` bounds. Returns the
/// number of bytes copied.
///
/// # Safety
///
/// Caller must ensure that `dst` and `dst_len` are valid for writing.
unsafe fn copy_bytes(dst: *mut u8, dst_len: usize, src: &[u8]) -> usize {
    let len = min(src.len(), dst_len);
    // SAFETY: since we have mutable access to `self` we know that `bytes`
    // can point to (part of) the same buffer as that would be UB already.
    // Furthermore we checked that the length doesn't overrun the buffer and
    // `parts_mut` impl must ensure that the `ptr` is valid.
    unsafe { dst.copy_from_nonoverlapping(src.as_ptr(), len) };
    len
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
/// use heph_rt::io::BufMut;
///
/// let mut buf = Vec::with_capacity(100);
/// buf.extend(b"Hello world!");
///
/// write_bytes(b" Hello mars!", &mut buf);
///
/// assert_eq!(&*buf, b"Hello world! Hello mars!");
///
/// fn write_bytes<B: BufMut>(src: &[u8], buf: &mut B) {
///     // Writes `src` to `buf`.
/// #   let (dst, len) = unsafe { buf.parts_mut() };
/// #   let len = std::cmp::min(src.len(), len);
/// #   // SAFETY: both the src and dst pointers are good. And we've ensured
/// #   // that the length is correct, not overwriting data we don't own or
/// #   // reading data we don't own.
/// #   unsafe {
/// #       std::ptr::copy_nonoverlapping(src.as_ptr(), dst, len);
/// #       buf.update_length(len);
/// #   }
/// }
/// ```
// SAFETY: `Vec<u8>` manages the allocation of the bytes, so as long as it's
// alive, so is the slice of bytes. When the `Vec`tor is leaked the allocation
// will also be leaked.
unsafe impl BufMut for Vec<u8> {
    unsafe fn parts_mut(&mut self) -> (*mut u8, usize) {
        let slice = self.spare_capacity_mut();
        (slice.as_mut_ptr().cast(), slice.len())
    }

    unsafe fn update_length(&mut self, n: usize) {
        let new = self.len() + n;
        debug_assert!(self.capacity() >= new);
        self.set_len(new);
    }

    fn spare_capacity(&self) -> usize {
        self.capacity() - self.len()
    }

    fn has_spare_capacity(&self) -> bool {
        self.capacity() != self.len()
    }
}

/// Trait that defines the behaviour of buffers used in reading using vectored
/// I/O, which requires mutable access.
///
/// # Notes
///
/// This trait can only be implemented by internal types. However it is already
/// implemented for arrays and tuples of buffers that implement [`BufMut`], [see below].
///
/// [see below]: #foreign-impls
///
/// # Safety
///
/// This has the same safety requirements as [`BufMut`], but then for all
/// buffers used.
pub trait BufMutSlice<const N: usize>: private::BufMutSlice<N> + 'static {
    /// Returns the total length of all buffer.s
    fn total_spare_capacity(&self) -> usize;

    /// Returns `true` at least one of the buffer has spare capacity.
    fn has_spare_capacity(&self) -> bool {
        self.total_spare_capacity() != 0
    }

    /// Extend the buffer with `bytes`, returns the number of bytes copied.
    fn extend_from_slice(&mut self, bytes: &[u8]) -> usize {
        let mut left = bytes;
        for iovec in unsafe { self.as_iovecs_mut() } {
            // SAFETY: `as_iovecs_mut` requirements are the same for `copy_bytes`.
            let n = unsafe { copy_bytes(iovec.iov_base.cast(), iovec.iov_len, left) };
            left = &left[n..];
            if left.is_empty() {
                break;
            }
        }
        let written = bytes.len() - left.len();
        // SAFETY: just written the bytes above.
        unsafe { self.update_length(written) };
        written
    }

    /// Wrap the buffer in `Limited`, which limits the amount of bytes used to
    /// `limit`.
    ///
    /// [`Limited::into_inner`] can be used to retrieve the buffer again,
    /// or a mutable reference to the buffer can be used and the limited buffer
    /// be dropped after usage.
    fn limit(self, limit: usize) -> Limited<Self>
    where
        Self: Sized,
    {
        Limited { buf: self, limit }
    }
}

// NOTE: see the `private` module below for the actual trait.

impl<B: BufMut, const N: usize> BufMutSlice<N> for [B; N] {
    fn total_spare_capacity(&self) -> usize {
        self.iter().map(BufMut::spare_capacity).sum()
    }

    fn has_spare_capacity(&self) -> bool {
        self.iter().any(BufMut::has_spare_capacity)
    }
}

// SAFETY: `BufMutSlice` has the same safety requirements as `BufMut` and since
// `B` implements `BufMut` it's safe to implement `BufMutSlice` for an array of
// `B`.
unsafe impl<B: BufMut, const N: usize> private::BufMutSlice<N> for [B; N] {
    unsafe fn as_iovecs_mut(&mut self) -> [libc::iovec; N] {
        let mut iovecs = MaybeUninit::uninit_array();
        for (buf, iovec) in self.iter_mut().zip(iovecs.iter_mut()) {
            let (ptr, len) = buf.parts_mut();
            _ = iovec.write(libc::iovec {
                iov_base: ptr.cast(),
                iov_len: len,
            });
        }
        MaybeUninit::array_assume_init(iovecs)
    }

    unsafe fn update_length(&mut self, n: usize) {
        let mut left = n;
        for buf in self {
            let (_, len) = buf.parts_mut();
            if len < left {
                // Fully initialised the buffer.
                buf.update_length(len);
                left -= len;
            } else {
                // Partially initialised the buffer.
                buf.update_length(left);
                return;
            }
        }
        unreachable!(
            "called BufMutSlice::set_init({n}), with buffers totaling in {} in size",
            n - left
        );
    }
}

// NOTE: Also see implementation of `BufMutSlice` for tuples in the macro
// `buf_slice_for_tuple` below.

/// Trait that defines the behaviour of buffers used in writing, which requires
/// read only access.
///
/// # Safety
///
/// Unlike normal buffers the buffer implementations for Heph have additional
/// requirements because Heph uses io_uring.
///
/// If the operation (that uses this buffer) is not polled to completion, i.e.
/// the `Future` is dropped before it returns `Poll::Ready`, the kernel still
/// has access to the buffer and will still attempt to read from it. This means
/// that we must delay deallocation in such a way that the kernel will not read
/// memory we don't have access to any more. This makes, for example, stack
/// based buffers unfit to implement `Buf`.  Because we can't delay the
/// deallocation once its dropped and the kernel will read part of your stack
/// (where the buffer used to be)! This would be a huge security risk.
#[allow(clippy::len_without_is_empty)]
pub unsafe trait Buf: 'static {
    /// Returns the reabable buffer as pointer and length parts.
    ///
    /// # Safety
    ///
    /// The implementation must ensure that the pointer is valid, i.e. not null
    /// and pointing to memory owned by the buffer. Furthermore it must ensure
    /// that the returned length is, in combination with the pointer, valid. In
    /// other words the memory the pointer and length are pointing to must be a
    /// valid memory address and owned by the buffer.
    unsafe fn parts(&self) -> (*const u8, usize);

    /// Returns itself as slice of bytes.
    ///
    /// # Implementation
    ///
    /// This cals [`Buf::parts`] and converts that into a slice. **Do not
    /// overwrite this method**.
    fn as_slice(&self) -> &[u8] {
        // SAFETY: the `Buf::parts` implementation ensures that the `ptr` and
        // `len` are valid, as well as the memory allocation they point to. So
        // creating a slice from it is safe.
        unsafe {
            let (ptr, len) = self.parts();
            slice::from_raw_parts(ptr, len)
        }
    }

    /// Length of the buffer in bytes.
    ///
    /// # Implementation
    ///
    /// This cals [`Buf::parts`] and returns the second part. **Do not overwrite
    /// this method**.
    fn len(&self) -> usize {
        // SAFETY: not using the pointer. The implementation of `Buf::parts`
        // must ensure the length is correct.
        unsafe { self.parts() }.1
    }

    /// Wrap the buffer in `Limited`, which limits the amount of bytes used to
    /// `limit`.
    ///
    /// [`Limited::into_inner`] can be used to retrieve the buffer again, or a
    /// mutable reference to the buffer can be used and the limited buffer be
    /// dropped after usage.
    fn limit(self, limit: usize) -> Limited<Self>
    where
        Self: Sized,
    {
        Limited { buf: self, limit }
    }
}

// SAFETY: `Vec<u8>` manages the allocation of the bytes, so as long as it's
// alive, so is the slice of bytes. When the `Vec`tor is leaked the allocation
// will also be leaked.
unsafe impl Buf for Vec<u8> {
    unsafe fn parts(&self) -> (*const u8, usize) {
        let slice = self.as_slice();
        (slice.as_ptr().cast(), <[u8]>::len(slice))
    }
}

// SAFETY: `String` is just a `Vec<u8>`, see it's implementation for the safety
// reasoning.
unsafe impl Buf for String {
    unsafe fn parts(&self) -> (*const u8, usize) {
        let slice = self.as_bytes();
        (slice.as_ptr().cast(), <[u8]>::len(slice))
    }
}

// SAFETY: `Box<[u8]>` manages the allocation of the bytes, so as long as it's
// alive, so is the slice of bytes. When the `Box` is leaked the allocation will
// also be leaked.
unsafe impl Buf for Box<[u8]> {
    unsafe fn parts(&self) -> (*const u8, usize) {
        let slice: &[u8] = self;
        (slice.as_ptr().cast(), <[u8]>::len(slice))
    }
}

// SAFETY: `Arc<[u8]>` manages the allocation of the bytes, so as long as it's
// alive, so is the slice of bytes. When the `Arc` is leaked the allocation will
// also be leaked.
unsafe impl Buf for Arc<[u8]> {
    unsafe fn parts(&self) -> (*const u8, usize) {
        let slice: &[u8] = self;
        (slice.as_ptr().cast(), <[u8]>::len(slice))
    }
}

// SAFETY: because the reference has a `'static` lifetime we know the bytes
// can't be deallocated, so it's safe to implement `Buf`.
unsafe impl Buf for &'static [u8] {
    unsafe fn parts(&self) -> (*const u8, usize) {
        (self.as_ptr(), <[u8]>::len(self))
    }
}

// SAFETY: because the reference has a `'static` lifetime we know the bytes
// can't be deallocated, so it's safe to implement `Buf`.
unsafe impl Buf for &'static str {
    unsafe fn parts(&self) -> (*const u8, usize) {
        (self.as_ptr(), <str>::len(self))
    }
}

// SAFETY: mix of `Vec<u8>` and `&'static [u8]`, see those Buf implementations
// for safety reasoning.
unsafe impl Buf for Cow<'static, [u8]> {
    unsafe fn parts(&self) -> (*const u8, usize) {
        (self.as_ptr(), <[u8]>::len(self))
    }
}

// SAFETY: mix of `String` and `&'static str`, see those Buf implementations for
// safety reasoning.
unsafe impl Buf for Cow<'static, str> {
    unsafe fn parts(&self) -> (*const u8, usize) {
        (self.as_ptr(), <str>::len(self))
    }
}

/// Trait that defines the behaviour of buffers used in writing using vectored
/// I/O, which requires read only access.
///
/// # Notes
///
/// This trait can only be implemented by internal types. However it is already
/// implemented for arrays and tuples of buffers that implement [`Buf`], [see
/// below].
///
/// [see below]: #foreign-impls
///
/// # Safety
///
/// This has the same safety requirements as [`Buf`], but then for all buffers
/// used.
pub trait BufSlice<const N: usize>: private::BufSlice<N> + 'static {
    /// Returns the reabable buffer as `IoSlice` structures.
    fn as_io_slices<'a>(&'a self) -> [IoSlice<'a>; N] {
        // SAFETY: `as_iovecs` requires the returned iovec to be valid.
        unsafe {
            self.as_iovecs().map(|iovec| {
                IoSlice::new(slice::from_raw_parts(iovec.iov_base.cast(), iovec.iov_len))
            })
        }
    }

    /// Returns the total length of all buffers in bytes.
    fn total_len(&self) -> usize {
        // SAFETY: `as_iovecs` requires the returned iovec to be valid.
        unsafe { self.as_iovecs().iter().map(|iovec| iovec.iov_len).sum() }
    }

    /// Wrap the buffer in `Limited`, which limits the amount of bytes used to
    /// `limit`.
    ///
    /// [`Limited::into_inner`] can be used to retrieve the buffer again,
    /// or a mutable reference to the buffer can be used and the limited buffer
    /// be dropped after usage.
    fn limit(self, limit: usize) -> Limited<Self>
    where
        Self: Sized,
    {
        Limited { buf: self, limit }
    }
}

// NOTE: see the `private` module below for the actual trait.

impl<B: Buf, const N: usize> BufSlice<N> for [B; N] {}

// SAFETY: `BufSlice` has the same safety requirements as `Buf` and since `B`
// implements `Buf` it's safe to implement `BufSlice` for an array of `B`.
unsafe impl<B: Buf, const N: usize> private::BufSlice<N> for [B; N] {
    unsafe fn as_iovecs(&self) -> [libc::iovec; N] {
        let mut iovecs = MaybeUninit::uninit_array();
        for (buf, iovec) in self.iter().zip(iovecs.iter_mut()) {
            let (ptr, len) = buf.parts();
            _ = iovec.write(libc::iovec {
                iov_base: ptr.cast::<libc::c_void>().cast_mut(),
                iov_len: len,
            });
        }
        MaybeUninit::array_assume_init(iovecs)
    }
}

macro_rules! buf_slice_for_tuple {
    (
        // Number of values.
        $N: expr,
        // Generic parameter name and tuple index.
        $( $generic: ident . $index: tt ),+
    ) => {
        impl<$( $generic: BufMut ),+> BufMutSlice<$N> for ($( $generic ),+) {
            fn total_spare_capacity(&self) -> usize {
                $( self.$index.spare_capacity() + )+
                0
            }

            fn has_spare_capacity(&self) -> bool {
                $( self.$index.has_spare_capacity() || )+
                false
            }
        }

        // SAFETY: `BufMutSlice` has the same safety requirements as `BufMut`
        // and since all generic buffers must implement `BufMut` it's safe to
        // implement `BufMutSlice` for a tuple of all those buffers.
        unsafe impl<$( $generic: BufMut ),+> private::BufMutSlice<$N> for ($( $generic ),+) {
            unsafe fn as_iovecs_mut(&mut self) -> [libc::iovec; $N] {
                [
                    $({
                        let (ptr, len) = self.$index.parts_mut();
                        libc::iovec {
                            iov_base: ptr.cast(),
                            iov_len: len,
                        }
                    }),+
                ]
            }

            unsafe fn update_length(&mut self, n: usize) {
                let mut left = n;
                $({
                    let (_, len) = self.$index.parts_mut();
                    if len < left {
                        // Fully initialised the buffer.
                        self.$index.update_length(len);
                        left -= len;
                    } else {
                        // Partially initialised the buffer.
                        self.$index.update_length(left);
                        return;
                    }
                })+
                unreachable!(
                    "called BufMutSlice::set_init({n}), with buffers totaling in {} in size",
                    n - left
                );
            }
        }

        impl<$( $generic: Buf ),+> BufSlice<$N> for ($( $generic ),+) { }

        // SAFETY: `BufSlice` has the same safety requirements as `Buf` and
        // since all generic buffers must implement `Buf` it's safe to implement
        // `BufSlice` for a tuple of all those buffers.
        unsafe impl<$( $generic: Buf ),+> private::BufSlice<$N> for ($( $generic ),+) {
            unsafe fn as_iovecs(&self) -> [libc::iovec; $N] {
                [
                    $({
                        let (ptr, len) = self.$index.parts();
                        libc::iovec {
                            iov_base: ptr as _,
                            iov_len: len,
                        }
                    }),+
                ]
            }
        }
    };
}

buf_slice_for_tuple!(2, A.0, B.1);
buf_slice_for_tuple!(3, A.0, B.1, C.2);
buf_slice_for_tuple!(4, A.0, B.1, C.2, D.3);
buf_slice_for_tuple!(5, A.0, B.1, C.2, D.3, E.4);
buf_slice_for_tuple!(6, A.0, B.1, C.2, D.3, E.4, F.5);
buf_slice_for_tuple!(7, A.0, B.1, C.2, D.3, E.4, F.5, G.6);
buf_slice_for_tuple!(8, A.0, B.1, C.2, D.3, E.4, F.5, G.6, I.7);

mod private {
    /// Private version of [`BufMutSlice`].
    ///
    /// [`BufMutSlice`]: crate::io::BufMutSlice
    ///
    /// # Safety
    ///
    /// See the [`a10::io::BufMutSlice`] trait.
    pub unsafe trait BufMutSlice<const N: usize>: 'static {
        /// Returns the writable buffers as `iovec` structures.
        ///
        /// # Safety
        ///
        /// This has the same safety requirements as [`BufMut::parts_mut`], but
        /// then for all buffers used.
        ///
        /// [`BufMut::parts_mut`]: crate::io::BufMut::parts_mut
        unsafe fn as_iovecs_mut(&mut self) -> [libc::iovec; N];

        /// Mark `n` bytes as initialised.
        ///
        /// # Safety
        ///
        /// The caller must ensure that `n` bytes are initialised in the vectors
        /// return by [`BufMutSlice::as_iovecs_mut`].
        ///
        /// The implementation must ensure that that proper buffer(s) are
        /// initialised. For example when this is called with `n = 10` with two
        /// buffers of size `8` the implementation should initialise the first
        /// buffer with `n = 8` and the second with `n = 10 - 8 = 2`.
        unsafe fn update_length(&mut self, n: usize);
    }

    /// Private version of [`BufSlice`].
    ///
    /// [`BufSlice`]: crate::io::BufSlice
    ///
    /// # Safety
    ///
    /// See the [`a10::io::BufSlice`] trait.
    pub unsafe trait BufSlice<const N: usize>: 'static {
        /// Returns the reabable buffer as `iovec` structures.
        ///
        /// # Safety
        ///
        /// This has the same safety requirements as [`Buf::parts`], but then for
        /// all buffers used.
        ///
        /// [`Buf::parts`]: crate::io::Buf::parts
        unsafe fn as_iovecs(&self) -> [libc::iovec; N];
    }
}

/// Wrapper around `B` to implement [`a10::io::BufMut`] and [`a10::io::Buf`].
pub(crate) struct BufWrapper<B>(pub(crate) B);

unsafe impl<B: BufMut> a10::io::BufMut for BufWrapper<B> {
    #[allow(clippy::cast_possible_truncation)]
    unsafe fn parts_mut(&mut self) -> (*mut u8, u32) {
        let (ptr, size) = self.0.parts_mut();
        (ptr, size as u32)
    }

    unsafe fn set_init(&mut self, n: usize) {
        self.0.update_length(n);
    }
}

unsafe impl<B: BufMut> BufMut for BufWrapper<B> {
    unsafe fn parts_mut(&mut self) -> (*mut u8, usize) {
        self.0.parts_mut()
    }

    unsafe fn update_length(&mut self, n: usize) {
        self.0.update_length(n);
    }

    fn spare_capacity(&self) -> usize {
        self.0.spare_capacity()
    }

    fn has_spare_capacity(&self) -> bool {
        self.0.has_spare_capacity()
    }
}

unsafe impl<B: Buf> a10::io::Buf for BufWrapper<B> {
    #[allow(clippy::cast_possible_truncation)]
    unsafe fn parts(&self) -> (*const u8, u32) {
        let (ptr, size) = self.0.parts();
        (ptr, size as u32)
    }
}

unsafe impl<B: Buf> Buf for BufWrapper<B> {
    unsafe fn parts(&self) -> (*const u8, usize) {
        self.0.parts()
    }
}

unsafe impl<B: BufMutSlice<N>, const N: usize> a10::io::BufMutSlice<N> for BufWrapper<B> {
    unsafe fn as_iovecs_mut(&mut self) -> [libc::iovec; N] {
        self.0.as_iovecs_mut()
    }

    unsafe fn set_init(&mut self, n: usize) {
        self.0.update_length(n);
    }
}

impl<B: BufMutSlice<N>, const N: usize> BufMutSlice<N> for BufWrapper<B> {
    fn total_spare_capacity(&self) -> usize {
        self.0.total_spare_capacity()
    }

    fn has_spare_capacity(&self) -> bool {
        self.0.has_spare_capacity()
    }
}

unsafe impl<B: BufMutSlice<N>, const N: usize> private::BufMutSlice<N> for BufWrapper<B> {
    unsafe fn as_iovecs_mut(&mut self) -> [libc::iovec; N] {
        self.0.as_iovecs_mut()
    }

    unsafe fn update_length(&mut self, n: usize) {
        self.0.update_length(n);
    }
}

unsafe impl<B: BufSlice<N>, const N: usize> a10::io::BufSlice<N> for BufWrapper<B> {
    unsafe fn as_iovecs(&self) -> [libc::iovec; N] {
        self.0.as_iovecs()
    }
}

impl<B: BufSlice<N>, const N: usize> BufSlice<N> for BufWrapper<B> {}

unsafe impl<B: BufSlice<N>, const N: usize> private::BufSlice<N> for BufWrapper<B> {
    unsafe fn as_iovecs(&self) -> [libc::iovec; N] {
        self.0.as_iovecs()
    }
}

/// Wrapper to limit the number of bytes `B` can use.
///
/// Created using [`Buf::limit`], [`BufMut::limit`], [`BufSlice::limit`] or
/// [`BufMutSlice::limit`].
#[derive(Debug)]
pub struct Limited<B> {
    buf: B,
    limit: usize,
}

impl<B> Limited<B> {
    /// Returns the underlying buffer.
    pub fn into_inner(self) -> B {
        self.buf
    }
}

unsafe impl<B: BufMut> BufMut for Limited<B> {
    unsafe fn parts_mut(&mut self) -> (*mut u8, usize) {
        let (ptr, size) = self.buf.parts_mut();
        (ptr, min(size, self.limit))
    }

    unsafe fn update_length(&mut self, n: usize) {
        self.limit -= n; // For use in read N bytes kind of calls.
        self.buf.update_length(n);
    }

    fn spare_capacity(&self) -> usize {
        min(self.buf.spare_capacity(), self.limit)
    }

    fn has_spare_capacity(&self) -> bool {
        self.limit != 0 && self.buf.has_spare_capacity()
    }
}

unsafe impl<B: Buf> Buf for Limited<B> {
    unsafe fn parts(&self) -> (*const u8, usize) {
        let (ptr, size) = self.buf.parts();
        (ptr, min(size, self.limit))
    }
}

impl<B: BufMutSlice<N>, const N: usize> BufMutSlice<N> for Limited<B> {
    fn total_spare_capacity(&self) -> usize {
        min(self.limit, self.buf.total_spare_capacity())
    }

    fn has_spare_capacity(&self) -> bool {
        self.limit != 0 && self.buf.has_spare_capacity()
    }
}

unsafe impl<B: BufMutSlice<N>, const N: usize> private::BufMutSlice<N> for Limited<B> {
    unsafe fn as_iovecs_mut(&mut self) -> [libc::iovec; N] {
        let mut total_len = 0;
        let mut iovecs = unsafe { self.buf.as_iovecs_mut() };
        for iovec in &mut iovecs {
            let n = total_len + iovec.iov_len;
            if n > self.limit {
                iovec.iov_len = self.limit - total_len;
                total_len = self.limit;
            } else {
                total_len = n;
            }
        }
        iovecs
    }

    unsafe fn update_length(&mut self, n: usize) {
        self.limit -= n; // For use in read N bytes kind of calls.
        self.buf.update_length(n);
    }
}

impl<B: BufSlice<N>, const N: usize> BufSlice<N> for Limited<B> {}

unsafe impl<B: BufSlice<N>, const N: usize> private::BufSlice<N> for Limited<B> {
    unsafe fn as_iovecs(&self) -> [libc::iovec; N] {
        let mut total_len = 0;
        let mut iovecs = unsafe { self.buf.as_iovecs() };
        for iovec in &mut iovecs {
            let n = total_len + iovec.iov_len;
            if n > self.limit {
                iovec.iov_len = self.limit - total_len;
                total_len = self.limit;
            } else {
                total_len = n;
            }
        }
        iovecs
    }
}
