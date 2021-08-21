use heph_http::head::version::UnknownVersion;
use heph_http::Version::{self, *};

use crate::assert_size;

#[test]
fn size() {
    assert_size::<Version>(1);
}

#[test]
fn major() {
    let tests = &[(Http10, 1), (Http11, 1)];
    for (version, expected) in tests {
        assert_eq!(version.major(), *expected);
    }
}

#[test]
fn minor() {
    let tests = &[(Http10, 0), (Http11, 1)];
    for (version, expected) in tests {
        assert_eq!(version.minor(), *expected);
    }
}

#[test]
fn highest_minor() {
    let tests = &[(Http10, Http11), (Http11, Http11)];
    for (version, expected) in tests {
        assert_eq!(version.highest_minor(), *expected);
    }
}

#[test]
fn from_str() {
    let tests = &[(Http10, "HTTP/1.0"), (Http11, "HTTP/1.1")];
    for (expected, input) in tests {
        let got: Version = input.parse().unwrap();
        assert_eq!(got, *expected);
        // NOTE: version (unlike most other types) is matched case-sensitive.
    }
    assert!("HTTP/1.2".parse::<Version>().is_err());
}

#[test]
fn as_str() {
    let tests = &[(Http10, "HTTP/1.0"), (Http11, "HTTP/1.1")];
    for (method, expected) in tests {
        assert_eq!(method.as_str(), *expected);
    }
}

#[test]
fn fmt_display() {
    let tests = &[(Http10, "HTTP/1.0"), (Http11, "HTTP/1.1")];
    for (method, expected) in tests {
        assert_eq!(*method.to_string(), **expected);
    }
}

#[test]
fn unknown_version_fmt_display() {
    assert_eq!(UnknownVersion.to_string(), "unknown HTTP version");
}
