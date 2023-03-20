//! Buffers.

use std::mem::MaybeUninit;

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
/// requirements because Heph uses I/O uring.
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
    /// [`parts`] are initialised.
    ///
    /// [`parts`]: BufMut::parts
    ///
    /// # Notes
    ///
    /// If this method is not implemented correctly methods such as
    /// [`TcpStream::recv_n`] will not work correctly (as the buffer will
    /// overwrite itself on successive reads).
    ///
    /// [`TcpStream::recv_n`]: crate::net::TcpStream::recv_n
    unsafe fn update_length(&mut self, n: usize);

    /// Returns the length of the buffer as returned by [`parts`].
    ///
    /// [`parts`]: BufMut::parts
    fn spare_capacity(&self) -> usize;

    /// Returns `true` if the buffer has spare capacity.
    fn has_spare_capacity(&self) -> bool {
        self.spare_capacity() == 0
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
/// use heph_rt::bytes::Bytes;
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
pub trait BufMutSlice<const N: usize>: private::BufMutSlice<N> + 'static {}

// NOTE: see the `private` module below for the actual trait.

impl<B: BufMut, const N: usize> BufMutSlice<N> for [B; N] {}

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
        for buf in self.iter_mut() {
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
/// requirements because Heph uses I/O uring.
///
/// If the operation (that uses this buffer) is not polled to completion, i.e.
/// the `Future` is dropped before it returns `Poll::Ready`, the kernel still
/// has access to the buffer and will still attempt to read from it. This means
/// that we must delay deallocation in such a way that the kernel will not read
/// memory we don't have access to any more. This makes, for example, stack
/// based buffers unfit to implement `Buf`.  Because we can't delay the
/// deallocation once its dropped and the kernel will read part of your stack
/// (where the buffer used to be)! This would be a huge security risk.
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
}

// SAFETY: `Vec<u8>` manages the allocation of the bytes, so as long as it's
// alive, so is the slice of bytes. When the `Vec`tor is leaked the allocation
// will also be leaked.
unsafe impl Buf for Vec<u8> {
    unsafe fn parts(&self) -> (*const u8, usize) {
        let slice = self.as_slice();
        (slice.as_ptr().cast(), slice.len())
    }
}

// SAFETY: `String` is just a `Vec<u8>`, see it's implementation for the safety
// reasoning.
unsafe impl Buf for String {
    unsafe fn parts(&self) -> (*const u8, usize) {
        let slice = self.as_bytes();
        (slice.as_ptr().cast(), slice.len())
    }
}

// SAFETY: because the reference has a `'static` lifetime we know the bytes
// can't be deallocated, so it's safe to implement `Buf`.
unsafe impl Buf for &'static [u8] {
    unsafe fn parts(&self) -> (*const u8, usize) {
        (self.as_ptr(), self.len())
    }
}

// SAFETY: because the reference has a `'static` lifetime we know the bytes
// can't be deallocated, so it's safe to implement `Buf`.
unsafe impl Buf for &'static str {
    unsafe fn parts(&self) -> (*const u8, usize) {
        (self.as_bytes().as_ptr(), self.len())
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
pub trait BufSlice<const N: usize>: private::BufSlice<N> + 'static {}

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
                iov_base: ptr as _,
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
        impl<$( $generic: BufMut ),+> BufMutSlice<$N> for ($( $generic ),+) { }
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
    pub unsafe trait BufMutSlice<const N: usize>: 'static {
        /// Returns the writable buffers as `iovec` structures.
        ///
        /// # Safety
        ///
        /// This has the same safety requirements as [`BufMut::parts`], but then for
        /// all buffers used.
        unsafe fn as_iovecs_mut(&mut self) -> [libc::iovec; N];

        /// Mark `n` bytes as initialised.
        ///
        /// # Safety
        ///
        /// The caller must ensure that `n` bytes are initialised in the vectors
        /// return by [`BufMutSlice::as_iovec`].
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
    pub unsafe trait BufSlice<const N: usize>: 'static {
        /// Returns the reabable buffer as `iovec` structures.
        ///
        /// # Safety
        ///
        /// This has the same safety requirements as [`Buf::parts`], but then for
        /// all buffers used.
        unsafe fn as_iovecs(&self) -> [libc::iovec; N];
    }
}

/// Wrapper around `B` to implement [`a10::io::BufMut`] and [`a10::io::Buf`].
pub(crate) struct BufWrapper<B>(pub(crate) B);

unsafe impl<B: BufMut> a10::io::BufMut for BufWrapper<B> {
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
        self.0.update_length(n)
    }
}

impl<B: BufMutSlice<N>, const N: usize> BufMutSlice<N> for BufWrapper<B> {}

unsafe impl<B: BufMutSlice<N>, const N: usize> private::BufMutSlice<N> for BufWrapper<B> {
    unsafe fn as_iovecs_mut(&mut self) -> [libc::iovec; N] {
        self.0.as_iovecs_mut()
    }

    unsafe fn update_length(&mut self, n: usize) {
        self.0.update_length(n)
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
