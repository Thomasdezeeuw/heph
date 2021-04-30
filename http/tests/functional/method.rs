use heph_http::Method::{self, *};

use crate::assert_size;

#[test]
fn size() {
    assert_size::<Method>(1);
}

#[test]
fn is_head() {
    assert!(Head.is_head());
    let tests = &[Get, Post, Put, Delete, Connect, Options, Trace, Patch];
    for method in tests {
        assert!(!method.is_head());
    }
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
