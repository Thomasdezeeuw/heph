//! Tests for the [`Bytes`] trait.

use std::cmp::min;
use std::ptr;

use heph::net::Bytes;

const DATA: &[u8] = b"Hello world!";
const DATA2: &[u8] = b"Hello mars.";

fn write_bytes<B>(src: &[u8], mut buf: B) -> usize
where
    B: Bytes,
{
    let dst = buf.as_bytes();
    let len = min(src.len(), dst.len());
    // Safety: both the src and dst pointers are good. And we've ensured
    // that the length is correct, not overwriting data we don't own or
    // reading data we don't own.
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr().cast(), len);
        buf.update_length(len);
    }
    len
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
