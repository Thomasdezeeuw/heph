use heph_http::head::method::UnknownMethod;
use heph_http::Method::{self, *};

use crate::assert_size;

#[test]
fn size() {
    assert_size::<Method>(1);
}

#[test]
fn is_safe() {
    let safe = &[Get, Head, Options, Trace];
    for method in safe {
        assert!(method.is_safe());
    }
    let not_safe = &[Post, Put, Delete, Connect, Patch];
    for method in not_safe {
        assert!(!method.is_safe());
    }
}

#[test]
fn is_idempotent() {
    let idempotent = &[Get, Head, Put, Delete, Options, Trace];
    for method in idempotent {
        assert!(method.is_idempotent());
    }
    let not_idempotent = &[Post, Connect, Patch];
    for method in not_idempotent {
        assert!(!method.is_idempotent());
    }
}

#[test]
fn expects_body() {
    let no_body = &[Head];
    for method in no_body {
        assert!(!method.expects_body());
    }
    let has_body = &[Get, Post, Put, Delete, Connect, Options, Trace, Patch];
    for method in has_body {
        assert!(method.expects_body());
    }
}

#[test]
fn as_str() {
    let tests = &[
        (Get, "GET"),
        (Head, "HEAD"),
        (Post, "POST"),
        (Put, "PUT"),
        (Delete, "DELETE"),
        (Connect, "CONNECT"),
        (Options, "OPTIONS"),
        (Trace, "TRACE"),
        (Patch, "PATCH"),
    ];
    for (method, expected) in tests {
        assert_eq!(method.as_str(), *expected);
    }
}

#[test]
fn from_str() {
    let tests = &[
        (Get, "GET"),
        (Head, "HEAD"),
        (Post, "POST"),
        (Put, "PUT"),
        (Delete, "DELETE"),
        (Connect, "CONNECT"),
        (Options, "OPTIONS"),
        (Trace, "TRACE"),
        (Patch, "PATCH"),
    ];
    for (expected, input) in tests {
        let got: Method = input.parse().unwrap();
        assert_eq!(got, *expected);
        // Must be case-insensitive.
        let got: Method = input.to_lowercase().parse().unwrap();
        assert_eq!(got, *expected);
    }
}

#[test]
fn from_invalid_str() {
    let tests = &["abc", "abcd", "abcde", "abcdef", "abcdefg", "abcdefgh"];
    for input in tests {
        assert!(input.parse::<Method>().is_err());
    }
}

#[test]
fn cmp_with_string() {
    let tests = &[
        (Get, "GET", true),
        (Head, "HEAD", true),
        (Post, "POST", true),
        (Put, "PUT", true),
        (Delete, "DELETE", true),
        (Connect, "CONNECT", true),
        (Options, "OPTIONS", true),
        (Trace, "TRACE", true),
        (Patch, "PATCH", true),
        // Case insensitive.
        (Get, "get", true),
        (Head, "Head", true),
        (Post, "posT", true),
        (Put, "put", true),
        (Delete, "delete", true),
        (Connect, "ConNECT", true),
        (Options, "OptionS", true),
        (Trace, "Trace", true),
        (Patch, "patcH", true),
        (Head, "POST", false),
        (Post, "head", false),
    ];
    for (status_code, str, expected) in tests {
        assert_eq!(status_code.eq(*str), *expected);
    }
}

#[test]
fn fmt_display() {
    let tests = &[
        (Get, "GET"),
        (Head, "HEAD"),
        (Post, "POST"),
        (Put, "PUT"),
        (Delete, "DELETE"),
        (Connect, "CONNECT"),
        (Options, "OPTIONS"),
        (Trace, "TRACE"),
        (Patch, "PATCH"),
    ];
    for (method, expected) in tests {
        assert_eq!(*method.to_string(), **expected);
    }
}

#[test]
fn unknown_method_fmt_display() {
    assert_eq!(UnknownMethod.to_string(), "unknown HTTP method");
}
