//! Tests for the [`Bytes`] trait.

use std::cmp::min;
use std::ptr;

use heph::net::{Bytes, BytesVectored};

const DATA: &[u8] = b"Hello world!";
const DATA2: &[u8] = b"Hello mars.";

fn write_bytes<B>(src: &[u8], mut buf: B) -> usize
where
    B: Bytes,
{
    let spare_capacity = buf.spare_capacity();
    let dst = buf.as_bytes();
    assert_eq!(dst.len(), spare_capacity);
    let len = min(src.len(), dst.len());
    // Safety: both the `src` and `dst` pointers are good. And we've ensured
    // that the length is correct, not overwriting data we don't own or reading
    // data we don't own.
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr().cast(), len);
        buf.update_length(len);
    }
    len
}

fn write_bytes_vectored<B>(src: &[u8], mut bufs: B) -> usize
where
    B: BytesVectored,
{
    let mut written = 0;
    let mut left = src;
    for buf in bufs.as_bufs().as_mut().iter_mut() {
        let len = min(left.len(), buf.len());
        // Safety: both the `left` and `dst` pointers are good. And we've
        // ensured that the length is correct, not overwriting data we don't own
        // or reading data we don't own.
        unsafe {
            ptr::copy_nonoverlapping(left.as_ptr(), buf.as_mut_ptr().cast(), len);
        }

        written += len;
        left = &left[len..];
        if left.is_empty() {
            break;
        }
    }
    unsafe { bufs.update_lengths(written) }
    written
}

#[test]
fn impl_for_vec() {
    let mut buf = Vec::<u8>::with_capacity(2 * DATA.len());
    assert_eq!(buf.spare_capacity(), 2 * DATA.len());
    assert!(buf.has_spare_capacity());
    let n = write_bytes(DATA, &mut buf);
    assert_eq!(n, DATA.len());
    assert_eq!(buf.len(), DATA.len());
    assert_eq!(&*buf, DATA);
    assert_eq!(buf.spare_capacity(), DATA.len());
    assert!(buf.has_spare_capacity());
}

#[test]
fn dont_overwrite_existing_bytes_in_vec() {
    let mut buf = Vec::<u8>::with_capacity(2 * DATA.len());
    assert_eq!(buf.spare_capacity(), 2 * DATA.len());
    assert!(buf.has_spare_capacity());
    buf.extend(DATA2);
    assert_eq!(buf.spare_capacity(), 2 * DATA.len() - DATA2.len());
    assert!(buf.has_spare_capacity());
    let start = buf.len();
    let n = write_bytes(DATA, &mut buf);
    assert_eq!(n, DATA.len());
    assert_eq!(buf.len(), DATA2.len() + DATA.len());
    assert_eq!(&buf[..start], DATA2); // Original bytes untouched.
    assert_eq!(&buf[start..start + n], DATA);
    assert_eq!(buf.spare_capacity(), 1);
    assert!(buf.has_spare_capacity());
    buf.push(b'a');
    assert_eq!(buf.spare_capacity(), 0);
    assert!(!buf.has_spare_capacity());
}

#[test]
fn limited_bytes() {
    const LIMIT: usize = 5;
    let mut buf = Vec::<u8>::with_capacity(2 * DATA.len()).limit(LIMIT);
    assert_eq!(buf.spare_capacity(), 5);
    assert!(buf.has_spare_capacity());

    let n = write_bytes(DATA, &mut buf);
    assert_eq!(n, LIMIT);
    assert_eq!(buf.spare_capacity(), 0);
    assert!(!buf.has_spare_capacity());
    let buf = buf.into_inner();
    assert_eq!(&*buf, &DATA[..LIMIT]);
    assert_eq!(buf.len(), LIMIT);
}

#[test]
fn vectored_array() {
    let mut bufs = [Vec::with_capacity(1), Vec::with_capacity(DATA.len())];
    assert_eq!(bufs.spare_capacity(), 1 + DATA.len());
    assert!(bufs.has_spare_capacity());
    let n = write_bytes_vectored(DATA, &mut bufs);
    assert_eq!(n, DATA.len());
    assert_eq!(bufs[0].len(), 1);
    assert_eq!(bufs[1].len(), DATA.len() - 1);
    assert_eq!(bufs[0], &DATA[..1]);
    assert_eq!(bufs[1], &DATA[1..]);
    assert_eq!(bufs.spare_capacity(), 1);
    assert!(bufs.has_spare_capacity());
    bufs[1].push(b'a');
    assert_eq!(bufs.spare_capacity(), 0);
    assert!(!bufs.has_spare_capacity());
}

#[test]
fn vectored_tuple() {
    let mut bufs = (
        Vec::with_capacity(1),
        Vec::with_capacity(3),
        Vec::with_capacity(DATA.len()),
    );
    assert_eq!(bufs.spare_capacity(), 1 + 3 + DATA.len());
    assert!(bufs.has_spare_capacity());
    let n = write_bytes_vectored(DATA, &mut bufs);
    assert_eq!(n, DATA.len());
    assert_eq!(bufs.0.len(), 1);
    assert_eq!(bufs.1.len(), 3);
    assert_eq!(bufs.2.len(), DATA.len() - 4);
    assert_eq!(bufs.0, &DATA[..1]);
    assert_eq!(bufs.1, &DATA[1..4]);
    assert_eq!(bufs.2, &DATA[4..]);
    assert_eq!(bufs.spare_capacity(), 4);
    assert!(bufs.has_spare_capacity());
    bufs.2.extend_from_slice(b"aaaa");
    assert_eq!(bufs.spare_capacity(), 0);
    assert!(!bufs.has_spare_capacity());
}

#[test]
fn limited_bytes_vectored() {
    const LIMIT: usize = 5;

    let mut bufs = [
        Vec::with_capacity(1),
        Vec::with_capacity(DATA.len()),
        Vec::with_capacity(10),
    ]
    .limit(LIMIT);
    assert_eq!(bufs.spare_capacity(), LIMIT);
    assert!(bufs.has_spare_capacity());

    let n = write_bytes_vectored(DATA, &mut bufs);
    assert_eq!(n, LIMIT);
    assert_eq!(bufs.spare_capacity(), 0);
    assert!(!bufs.has_spare_capacity());
    let bufs = bufs.into_inner();
    assert_eq!(bufs[0].len(), 1);
    assert_eq!(bufs[1].len(), LIMIT - 1);
    assert_eq!(bufs[2].len(), 0);
    assert_eq!(bufs[0], &DATA[..1]);
    assert_eq!(bufs[1], &DATA[1..LIMIT]);
    assert_eq!(bufs[2], &[]);
}
