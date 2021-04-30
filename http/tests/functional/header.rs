use heph_http::header::{Header, HeaderName, Headers};

use crate::assert_size;

#[test]
fn sizes() {
    assert_size::<Headers>(48);
    assert_size::<Header>(48);
    assert_size::<HeaderName<'static>>(32);
}

#[test]
fn new_header() {
    const _MY_HEADER: Header<'static, 'static> =
        Header::new(HeaderName::USER_AGENT, b"Heph-HTTP/0.1");
    let _header = Header::new(HeaderName::USER_AGENT, b"");
    // Should be fine.
    let _header = Header::new(HeaderName::USER_AGENT, b"\rabc\n");
}

#[test]
#[should_panic = "header value contains CRLF ('\\r\\n')"]
fn new_header_with_crlf_should_panic() {
    let _header = Header::new(HeaderName::USER_AGENT, b"\r\n");
}

#[test]
#[should_panic = "header value contains CRLF ('\\r\\n')"]
fn new_header_with_crlf_should_panic2() {
    let _header = Header::new(HeaderName::USER_AGENT, b"some_text\r\n");
}

#[test]
fn parse_header() {
    const LENGTH: Header<'static, 'static> = Header::new(HeaderName::CONTENT_LENGTH, b"100");
    assert_eq!(LENGTH.parse::<usize>().unwrap(), 100);
}

#[test]
fn from_str_known_headers() {
    let known_headers = &[
        "allow",
        "user-agent",
        "x-request-id",
        "content-length",
        "transfer-encoding",
    ];
    for name in known_headers {
        let header_name = HeaderName::from_str(name);
        assert!(!header_name.is_heap_allocated(), "header: {}", name);
    }
}

#[test]
fn from_str_custom() {
    let unknown_headers = &["my-header", "My-Header"];
    for name in unknown_headers {
        let header_name = HeaderName::from_str(name);
        assert!(header_name.is_heap_allocated(), "header: {}", name);
        assert_eq!(header_name, "my-header");
        assert_eq!(header_name.as_ref(), "my-header");
    }

    let name = "bllow"; // Matches length of "Allow" header.
    let header_name = HeaderName::from_str(name);
    assert!(header_name.is_heap_allocated(), "header: {}", name);
}

#[test]
fn from_lowercase() {
    let header_name = HeaderName::from_lowercase("my-header");
    assert_eq!(header_name, "my-header");
    assert_eq!(header_name.as_ref(), "my-header");
}

#[test]
#[should_panic = "header name not lowercase"]
fn from_lowercase_not_lowercase_should_panic() {
    let _name = HeaderName::from_lowercase("My-Header");
}

#[test]
fn from_string() {
    let header_name = HeaderName::from("my-header".to_owned());
    assert_eq!(header_name, "my-header");
    assert_eq!(header_name.as_ref(), "my-header");
}

#[test]
fn from_string_makes_lowercase() {
    let header_name = HeaderName::from("My-Header".to_owned());
    assert_eq!(header_name, "my-header");
    assert_eq!(header_name.as_ref(), "my-header");
}

#[test]
fn compare_is_case_insensitive() {
    let tests = &[
        HeaderName::from_lowercase("my-header"),
        HeaderName::from("My-Header".to_owned()),
    ];
    for header_name in tests {
        assert_eq!(header_name, "my-header");
        assert_eq!(header_name, "My-Header");
        assert_eq!(header_name, "mY-hEaDeR");
        assert_eq!(header_name.as_ref(), "my-header");
    }
    assert_eq!(tests[0], tests[1]);
}

#[test]
fn fmt_display() {
    let tests = &[
        HeaderName::from_lowercase("my-header"),
        HeaderName::from("My-Header".to_owned()),
    ];
    for header_name in tests {
        assert_eq!(header_name.to_string(), "my-header");
    }
}
