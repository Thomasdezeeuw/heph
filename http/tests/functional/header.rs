use std::fmt;

use heph_http::header::{Header, HeaderName, Headers};
use heph_http::FromBytes;

use crate::assert_size;

#[test]
fn sizes() {
    assert_size::<Headers>(48);
    assert_size::<Header>(48);
    assert_size::<HeaderName<'static>>(32);
}

#[test]
fn headers_add_one_header() {
    const VALUE: &[u8] = b"GET";

    let mut headers = Headers::EMPTY;
    headers.add(Header::new(HeaderName::ALLOW, VALUE));
    assert_eq!(headers.len(), 1);

    check_header(&headers, &HeaderName::ALLOW, VALUE, "GET");
    check_iter(&headers, &[(HeaderName::ALLOW, VALUE)]);
}

#[test]
fn headers_add_multiple_headers() {
    const ALLOW: &[u8] = b"GET";
    const CONTENT_LENGTH: &[u8] = b"123";
    const X_REQUEST_ID: &[u8] = b"abc-def";

    let mut headers = Headers::EMPTY;
    headers.add(Header::new(HeaderName::ALLOW, ALLOW));
    headers.add(Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH));
    headers.add(Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID));
    assert_eq!(headers.len(), 3);

    check_header(&headers, &HeaderName::ALLOW, ALLOW, "GET");
    check_header(&headers, &HeaderName::CONTENT_LENGTH, CONTENT_LENGTH, 123);
    check_header(&headers, &HeaderName::X_REQUEST_ID, X_REQUEST_ID, "abc-def");
    check_iter(
        &headers,
        &[
            (HeaderName::ALLOW, ALLOW),
            (HeaderName::CONTENT_LENGTH, CONTENT_LENGTH),
            (HeaderName::X_REQUEST_ID, X_REQUEST_ID),
        ],
    );
}

#[test]
fn headers_from_header() {
    const VALUE: &[u8] = b"GET";
    let header = Header::new(HeaderName::ALLOW, VALUE);
    let headers = Headers::from(header.clone());
    assert_eq!(headers.len(), 1);

    check_header(&headers, &HeaderName::ALLOW, VALUE, "GET");
    check_iter(&headers, &[(HeaderName::ALLOW, VALUE)]);
}

fn check_header<'a, T>(
    headers: &'a Headers,
    name: &'_ HeaderName<'_>,
    value: &'_ [u8],
    parsed_value: T,
) where
    T: FromBytes<'a> + PartialEq + fmt::Debug,
    <T as FromBytes<'a>>::Err: fmt::Debug,
{
    let got = headers.get(name).unwrap();
    assert_eq!(got.name(), name);
    assert_eq!(got.value(), value);
    assert_eq!(got.parse::<T>().unwrap(), parsed_value);

    assert_eq!(headers.get_value(name).unwrap(), value);
}

fn check_iter(headers: &'_ Headers, expected: &[(HeaderName<'_>, &'_ [u8])]) {
    let mut len = expected.len();
    let mut iter = headers.iter();
    assert_eq!(iter.len(), len);
    assert_eq!(iter.size_hint(), (len, Some(len)));
    for (name, value) in expected {
        let got = iter.next().unwrap();
        assert_eq!(got.name(), name);
        assert_eq!(got.value(), *value);
        len -= 1;
        assert_eq!(iter.len(), len);
        assert_eq!(iter.size_hint(), (len, Some(len)));
    }
    assert_eq!(iter.count(), 0);

    let iter = headers.iter();
    assert_eq!(iter.count(), expected.len());
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

        // Matching should be case-insensitive.
        let header_name = HeaderName::from_str(&name.to_uppercase());
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
