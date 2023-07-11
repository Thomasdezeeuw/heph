//! Tests for the io module.

use std::borrow::Cow;
use std::cmp::min;
use std::ptr;
use std::sync::Arc;

use heph_rt::io::{Buf, BufMut, BufMutSlice, BufSlice};

const DATA: &[u8] = b"Hello world!";
const DATA2: &[u8] = b"Hello mars.";

fn write_bytes<B: BufMut>(src: &[u8], buf: &mut B) -> usize {
    let spare_capacity = buf.spare_capacity();
    let (dst, len) = unsafe { buf.parts_mut() };
    assert_eq!(len, spare_capacity);
    let len = min(src.len(), len);
    // SAFETY: both the `src` and `dst` pointers are good. And we've ensured
    // that the length is correct, not overwriting data we don't own or reading
    // data we don't own.
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr(), dst, len);
        buf.update_length(len);
    }
    len
}

fn write_bytes_vectored<B: BufMutSlice<N>, const N: usize>(src: &[u8], bufs: &mut B) -> usize {
    let mut written = 0;
    let mut left = src;
    for iovec in unsafe { bufs.as_iovecs_mut() } {
        let len = min(left.len(), iovec.iov_len);
        // SAFETY: both the `left` and `dst` pointers are good. And we've
        // ensured that the length is correct, not overwriting data we don't own
        // or reading data we don't own.
        unsafe {
            ptr::copy_nonoverlapping(left.as_ptr(), iovec.iov_base.cast(), len);
        }

        written += len;
        left = &left[len..];
        if left.is_empty() {
            break;
        }
    }
    unsafe { bufs.update_length(written) }
    written
}

#[test]
fn buf_mut_for_vec() {
    test_buf_mut(Vec::with_capacity(TEST_BUF_MUT_CAPACITY));
}

#[test]
fn buf_mut_for_limited() {
    let buf = Vec::with_capacity(TEST_BUF_MUT_CAPACITY + 10);
    test_buf_mut(BufMut::limit(buf, TEST_BUF_MUT_CAPACITY));
}

const TEST_BUF_MUT_CAPACITY: usize = DATA.len() + DATA2.len();

fn test_buf_mut<B: BufMut>(mut buf: B) {
    let capacity = TEST_BUF_MUT_CAPACITY;
    let (_, len) = unsafe { buf.parts_mut() };
    assert_eq!(len, capacity);
    assert_eq!(buf.spare_capacity(), capacity);
    assert!(buf.has_spare_capacity());

    let written = write_bytes(DATA, &mut buf);
    let capacity_left = capacity - written;
    let (_, len) = unsafe { buf.parts_mut() };
    assert_eq!(len, capacity_left);
    assert_eq!(buf.spare_capacity(), capacity_left);
    assert!(buf.has_spare_capacity());

    let written = write_bytes(DATA2, &mut buf);
    assert_eq!(written, capacity_left);
    let (_, len) = unsafe { buf.parts_mut() };
    assert_eq!(len, 0);
    assert_eq!(buf.spare_capacity(), 0);
    assert!(!buf.has_spare_capacity());
}

#[test]
fn buf_mut_extend_from_slice() {
    let mut buf = Vec::with_capacity(DATA.len() + DATA2.len() + 2);

    let n = BufMut::extend_from_slice(&mut buf, DATA);
    assert_eq!(n, DATA.len());
    assert_eq!(buf, DATA);

    let n = BufMut::extend_from_slice(&mut buf, DATA2);
    assert_eq!(n, DATA2.len());
    assert_eq!(&buf[DATA.len()..], DATA2);
}

#[test]
fn buf_for_vec() {
    test_buf(Vec::from(DATA))
}

#[test]
fn buf_for_string() {
    test_buf(String::from(std::str::from_utf8(DATA).unwrap()))
}

#[test]
fn buf_for_boxed_slice() {
    test_buf(Box::<[u8]>::from(DATA))
}

#[test]
fn buf_for_arc_slice() {
    test_buf(Arc::<[u8]>::from(DATA))
}

#[test]
fn buf_for_static_slice() {
    test_buf(DATA)
}

#[test]
fn buf_for_static_str() {
    test_buf(DATA)
}

#[test]
fn buf_for_static_cow() {
    test_buf::<Cow<'static, [u8]>>(Cow::Borrowed(DATA));
    test_buf::<Cow<'static, [u8]>>(Cow::Owned(Vec::from(DATA)));
}

#[test]
fn buf_for_static_cow_str() {
    test_buf::<Cow<'static, str>>(Cow::Borrowed("Hello world!"));
    test_buf::<Cow<'static, str>>(Cow::Owned(String::from("Hello world!")));
}

#[test]
fn buf_for_limited() {
    test_buf(DATA.limit(DATA.len())); // Same length.
    test_buf(DATA.limit(DATA.len() + 1)); // Larger.
    let mut buf = Vec::new();
    buf.push(DATA);
    buf.push(DATA2);
    test_buf(DATA.limit(DATA.len())); // Smaller.
}

fn test_buf<B: Buf>(buf: B) {
    let (ptr, len) = unsafe { buf.parts() };
    let got = unsafe { std::slice::from_raw_parts(ptr, len) };
    assert_eq!(got, DATA);
}

#[test]
fn buf_slice_as_io_slices() {
    test_buf_slice([DATA, DATA2]);
    test_buf_slice([Vec::from(DATA), Vec::from(DATA2)]);
    test_buf_slice((DATA, Vec::from(DATA2)));
}

#[test]
fn buf_slice_for_limited() {
    test_buf_slice([DATA, DATA2].limit(DATA.len() + DATA2.len())); // Same length.
    test_buf_slice([DATA, DATA2].limit(DATA.len() + DATA2.len() + 1)); // Larger.
    let buf0 = Vec::from(DATA);
    let mut buf1 = Vec::with_capacity(30);
    buf1.extend_from_slice(DATA2);
    buf1.resize(DATA2.len() * 2, 0);
    test_buf_slice(BufSlice::limit([buf0, buf1], DATA.len() + DATA2.len())); // Smaller.
}

fn test_buf_slice<B: BufSlice<2>>(buf: B) {
    let [got0, got1] = buf.as_io_slices();
    assert_eq!(&*got0, DATA);
    assert_eq!(&*got1, DATA2);
    assert_eq!(buf.total_len(), got0.len() + got1.len());
}

#[test]
fn buf_mut_slice_for_array() {
    let mut bufs = [Vec::with_capacity(1), Vec::with_capacity(DATA.len())];
    assert_eq!(bufs.total_spare_capacity(), 1 + DATA.len());
    assert!(bufs.has_spare_capacity());
    let n = write_bytes_vectored(DATA, &mut bufs);
    assert_eq!(n, DATA.len());
    assert_eq!(bufs[0].len(), 1);
    assert_eq!(bufs[1].len(), DATA.len() - 1);
    assert_eq!(bufs[0], &DATA[..1]);
    assert_eq!(bufs[1], &DATA[1..]);
    assert_eq!(bufs.total_spare_capacity(), 1);
    assert!(bufs.has_spare_capacity());
    bufs[1].push(b'a');
    assert_eq!(bufs.total_spare_capacity(), 0);
    assert!(!bufs.has_spare_capacity());
}

#[test]
fn buf_mut_slice_for_tuple() {
    let mut bufs = (
        Vec::with_capacity(1),
        Vec::with_capacity(3),
        Vec::with_capacity(DATA.len()),
    );
    assert_eq!(bufs.total_spare_capacity(), 1 + 3 + DATA.len());
    assert!(bufs.has_spare_capacity());
    let n = write_bytes_vectored(DATA, &mut bufs);
    assert_eq!(n, DATA.len());
    assert_eq!(bufs.0.len(), 1);
    assert_eq!(bufs.1.len(), 3);
    assert_eq!(bufs.2.len(), DATA.len() - 4);
    assert_eq!(bufs.0, &DATA[..1]);
    assert_eq!(bufs.1, &DATA[1..4]);
    assert_eq!(bufs.2, &DATA[4..]);
    assert_eq!(bufs.total_spare_capacity(), 4);
    assert!(bufs.has_spare_capacity());
    bufs.2.extend_from_slice(b"aaaa");
    assert_eq!(bufs.total_spare_capacity(), 0);
    assert!(!bufs.has_spare_capacity());
}

#[test]
fn buf_mut_slice_extend_from_slice() {
    let buf1 = Vec::with_capacity(DATA.len());
    let buf2 = Vec::with_capacity(DATA2.len() + 1);
    let mut bufs = [buf1, buf2];

    let n = BufMutSlice::extend_from_slice(&mut bufs, DATA);
    assert_eq!(n, DATA.len());
    assert_eq!(bufs[0], DATA);

    let n = BufMutSlice::extend_from_slice(&mut bufs, DATA2);
    assert_eq!(n, DATA2.len());
    assert_eq!(bufs[1], DATA2);
}

#[test]
fn buf_mut_slice_for_limited() {
    const TARGET_LENGTH: usize = DATA.len() + DATA2.len();
    // Same length.
    test_buf_mut_slice(
        BufMutSlice::limit(
            [
                Vec::with_capacity(DATA.len()),
                Vec::with_capacity(DATA2.len()),
            ],
            TARGET_LENGTH,
        ),
        TARGET_LENGTH,
    );
    // Larger.
    test_buf_mut_slice(
        BufMutSlice::limit(
            [
                Vec::with_capacity(DATA.len()),
                Vec::with_capacity(DATA2.len()),
            ],
            TARGET_LENGTH + 1,
        ),
        TARGET_LENGTH,
    );
    // Smaller.
    test_buf_mut_slice(
        BufMutSlice::limit(
            [
                Vec::with_capacity(DATA.len()),
                Vec::with_capacity(DATA2.len()),
            ],
            TARGET_LENGTH - 1,
        ),
        TARGET_LENGTH - 1,
    );
}

fn test_buf_mut_slice<B: BufMutSlice<2>>(mut bufs: B, expected_limit: usize) {
    let total_capacity = bufs.total_spare_capacity();
    assert_eq!(total_capacity, expected_limit);
    let n = bufs.extend_from_slice(DATA);
    let m = bufs.extend_from_slice(DATA2);
    assert!(total_capacity <= n + m);
    assert!(!bufs.has_spare_capacity());
    assert!(bufs.total_spare_capacity() == 0);
}
