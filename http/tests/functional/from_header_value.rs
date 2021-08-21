use std::fmt;
use std::time::SystemTime;

use heph_http::head::header::{FromHeaderValue, ParseIntError, ParseTimeError};

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

    test_parse(b"255", u8::MAX);
    test_parse(b"65535", u16::MAX);
    test_parse(b"4294967295", u32::MAX);
    test_parse(b"18446744073709551615", u64::MAX);
    test_parse(b"18446744073709551615", usize::MAX);
}

#[test]
fn integers_overflow() {
    // In multiplication.
    test_parse_fail::<u8>(b"300");
    test_parse_fail::<u16>(b"70000");
    test_parse_fail::<u32>(b"5000000000");
    test_parse_fail::<u64>(b"20000000000000000000");
    test_parse_fail::<usize>(b"20000000000000000000");

    // In addition.
    test_parse_fail::<u8>(b"257");
    test_parse_fail::<u16>(b"65537");
    test_parse_fail::<u32>(b"4294967297");
    test_parse_fail::<u64>(b"18446744073709551616");
    test_parse_fail::<usize>(b"18446744073709551616");
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

#[test]
fn system_time() {
    test_parse(b"Thu, 01 Jan 1970 00:00:00 GMT", SystemTime::UNIX_EPOCH); // IMF-fixdate.
    test_parse(b"Thursday, 01-Jan-70 00:00:00 GMT", SystemTime::UNIX_EPOCH); // RFC 850.
    test_parse(b"Thu Jan  1 00:00:00 1970", SystemTime::UNIX_EPOCH); // ANSI C’s `asctime`.
}

#[test]
fn invalid_system_time() {
    test_parse_fail::<SystemTime>(b"\xa0\xa1"); // Invalid UTF-8.
    test_parse_fail::<SystemTime>(b"ABC, 01 Jan 1970 00:00:00 GMT"); // Invalid format.
}

#[track_caller]
fn test_parse<'a, T>(value: &'a [u8], expected: T)
where
    T: FromHeaderValue<'a> + fmt::Debug + PartialEq,
    <T as FromHeaderValue<'a>>::Err: fmt::Debug,
{
    assert_eq!(T::from_bytes(value).unwrap(), expected);
}

#[track_caller]
fn test_parse_fail<'a, T>(value: &'a [u8])
where
    T: FromHeaderValue<'a> + fmt::Debug + PartialEq,
    <T as FromHeaderValue<'a>>::Err: fmt::Debug,
{
    assert!(T::from_bytes(value).is_err());
}

#[test]
fn parse_int_error_fmt_display() {
    assert_eq!(ParseIntError.to_string(), "invalid integer");
}

#[test]
fn parse_time_error_fmt_display() {
    assert_eq!(ParseTimeError.to_string(), "invalid time");
}
