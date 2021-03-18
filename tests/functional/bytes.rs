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
    let dst = buf.as_bytes();
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
    let n = write_bytes(DATA, &mut buf);
    assert_eq!(n, DATA.len());
    assert_eq!(buf.len(), DATA.len());
    assert_eq!(&*buf, DATA);
}

#[test]
fn dont_overwrite_existing_bytes_in_vec() {
    let mut buf = Vec::<u8>::with_capacity(2 * DATA.len());
    buf.extend(DATA2);
    let start = buf.len();
    let n = write_bytes(DATA, &mut buf);
    assert_eq!(n, DATA.len());
    assert_eq!(buf.len(), DATA2.len() + DATA.len());
    assert_eq!(&buf[..start], DATA2); // Original bytes untouched.
    assert_eq!(&buf[start..start + n], DATA);
}

#[test]
fn vectored_array() {
    let mut bufs = [Vec::with_capacity(1), Vec::with_capacity(DATA.len())];
    let n = write_bytes_vectored(DATA, &mut bufs);
    assert_eq!(n, DATA.len());
    assert_eq!(bufs[0].len(), 1);
    assert_eq!(bufs[1].len(), DATA.len() - 1);
    assert_eq!(bufs[0], &DATA[..1]);
    assert_eq!(bufs[1], &DATA[1..]);
}

#[test]
fn vectored_tuple() {
    let mut bufs = (
        Vec::with_capacity(1),
        Vec::with_capacity(3),
        Vec::with_capacity(DATA.len()),
    );
    let n = write_bytes_vectored(DATA, &mut bufs);
    assert_eq!(n, DATA.len());
    assert_eq!(bufs.0.len(), 1);
    assert_eq!(bufs.1.len(), 3);
    assert_eq!(bufs.2.len(), DATA.len() - 4);
    assert_eq!(bufs.0, &DATA[..1]);
    assert_eq!(bufs.1, &DATA[1..4]);
    assert_eq!(bufs.2, &DATA[4..]);
}
