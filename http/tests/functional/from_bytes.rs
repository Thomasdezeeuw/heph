use std::fmt;

use heph_http::FromBytes;

#[test]
fn str() {
    test_parse(b"123", "123");
    test_parse(b"abc", "abc");
}

#[test]
fn str_not_utf8() {
    test_parse_fail::<&str>(&[0, 255]);
}

#[test]
fn integers() {
    test_parse(b"123", 123u8);
    test_parse(b"123", 123u16);
    test_parse(b"123", 123u32);
    test_parse(b"123", 123u64);
    test_parse(b"123", 123usize);
}

#[test]
fn integers_overflow() {
    test_parse_fail::<u8>(b"256");
    test_parse_fail::<u16>(b"65536");
    test_parse_fail::<u32>(b"4294967296");
    test_parse_fail::<u64>(b"18446744073709551615");
    test_parse_fail::<usize>(b"18446744073709551615");
}

#[test]
fn empty_integers() {
    test_parse_fail::<u8>(b"");
    test_parse_fail::<u16>(b"");
    test_parse_fail::<u32>(b"");
    test_parse_fail::<u64>(b"");
    test_parse_fail::<usize>(b"");
}

#[test]
fn invalid_integers() {
    test_parse_fail::<u8>(b"abc");
    test_parse_fail::<u16>(b"abc");
    test_parse_fail::<u32>(b"abc");
    test_parse_fail::<u64>(b"abc");
    test_parse_fail::<usize>(b"abc");

    test_parse_fail::<u8>(b"2a");
    test_parse_fail::<u16>(b"2a");
    test_parse_fail::<u32>(b"2a");
    test_parse_fail::<u64>(b"2a");
    test_parse_fail::<usize>(b"2a");
}

fn test_parse<'a, T>(value: &'a [u8], expected: T)
where
    T: FromBytes<'a> + fmt::Debug + PartialEq,
    <T as FromBytes<'a>>::Err: fmt::Debug,
{
    assert_eq!(T::from_bytes(value).unwrap(), expected);
}

fn test_parse_fail<'a, T>(value: &'a [u8])
where
    T: FromBytes<'a> + fmt::Debug + PartialEq,
    <T as FromBytes<'a>>::Err: fmt::Debug,
{
    assert!(T::from_bytes(value).is_err());
}
