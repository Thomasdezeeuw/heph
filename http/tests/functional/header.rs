use std::fmt;
use std::iter::FromIterator;

use heph_http::head::header::{FromHeaderValue, Header, HeaderName, Headers};

use crate::assert_size;

#[test]
fn sizes() {
    assert_size::<Headers>(48);
    assert_size::<Header>(32);
    assert_size::<HeaderName<'static>>(16);
}

#[test]
fn headers_append_one_header() {
    const VALUE: &[u8] = b"GET";

    let mut headers = Headers::EMPTY;
    headers.append(Header::new(HeaderName::ALLOW, VALUE));
    assert_eq!(headers.len(), 1);
    assert!(!headers.is_empty());

    check_header(&headers, &HeaderName::ALLOW, VALUE, "GET");
    check_iter(&headers, &[(HeaderName::ALLOW, VALUE)]);
}

#[test]
fn headers_append_multiple_headers() {
    const ALLOW: &[u8] = b"GET";
    const CONTENT_LENGTH: &[u8] = b"123";
    const X_REQUEST_ID: &[u8] = b"abc-def";

    let mut headers = Headers::EMPTY;
    headers.append(Header::new(HeaderName::ALLOW, ALLOW));
    headers.append(Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH));
    headers.append(Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID));
    assert_eq!(headers.len(), 3);
    assert!(!headers.is_empty());

    check_header(&headers, &HeaderName::ALLOW, ALLOW, "GET");
    #[rustfmt::skip]
    check_header(&headers, &HeaderName::CONTENT_LENGTH, CONTENT_LENGTH, 123usize);
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
fn headers_insert() {
    const ALLOW: &[u8] = b"GET";
    const CONTENT_LENGTH: &[u8] = b"123";
    const X_REQUEST_ID: &[u8] = b"abc-def";

    let mut headers = Headers::EMPTY;
    headers.append(Header::new(HeaderName::ALLOW, ALLOW));
    headers.append(Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH));
    headers.append(Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH));
    headers.append(Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID));
    headers.append(Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID));
    headers.append(Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID));
    assert_eq!(headers.len(), 6);
    assert!(!headers.is_empty());

    // Should overwrite the headers appended above.
    headers.insert(Header::new(HeaderName::ALLOW, ALLOW));
    headers.insert(Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH));
    headers.insert(Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID));
    assert_eq!(headers.len(), 3);
    assert!(!headers.is_empty());

    check_header(&headers, &HeaderName::ALLOW, ALLOW, "GET");
    #[rustfmt::skip]
    check_header(&headers, &HeaderName::CONTENT_LENGTH, CONTENT_LENGTH, 123usize);
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
    let headers = Headers::from(header);
    assert_eq!(headers.len(), 1);
    assert!(!headers.is_empty());

    check_header(&headers, &HeaderName::ALLOW, VALUE, "GET");
    check_iter(&headers, &[(HeaderName::ALLOW, VALUE)]);
}

#[test]
fn headers_from_array() {
    const ALLOW: &[u8] = b"GET";
    const CONTENT_LENGTH: &[u8] = b"123";
    const X_REQUEST_ID: &[u8] = b"abc-def";

    let headers = Headers::from([
        Header::new(HeaderName::ALLOW, ALLOW),
        Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH),
        Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID),
    ]);
    assert_eq!(headers.len(), 3);
    assert!(!headers.is_empty());

    check_header(&headers, &HeaderName::ALLOW, ALLOW, "GET");
    #[rustfmt::skip]
    check_header(&headers, &HeaderName::CONTENT_LENGTH, CONTENT_LENGTH, 123usize);
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
fn headers_from_slice() {
    const ALLOW: &[u8] = b"GET";
    const CONTENT_LENGTH: &[u8] = b"123";
    const X_REQUEST_ID: &[u8] = b"abc-def";

    let expected_headers: &[_] = &[
        Header::new(HeaderName::ALLOW, ALLOW),
        Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH),
        Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID),
    ];

    let headers = Headers::from(expected_headers);
    assert_eq!(headers.len(), 3);
    assert!(!headers.is_empty());

    check_header(&headers, &HeaderName::ALLOW, ALLOW, "GET");
    #[rustfmt::skip]
    check_header(&headers, &HeaderName::CONTENT_LENGTH, CONTENT_LENGTH, 123usize);
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
fn headers_from_iter_and_extend() {
    const ALLOW: &[u8] = b"GET";
    const CONTENT_LENGTH: &[u8] = b"123";
    const X_REQUEST_ID: &[u8] = b"abc-def";

    let mut headers = Headers::from_iter([
        Header::new(HeaderName::ALLOW, ALLOW),
        Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH),
    ]);
    assert_eq!(headers.len(), 2);
    assert!(!headers.is_empty());

    headers.extend([Header::new(HeaderName::X_REQUEST_ID, X_REQUEST_ID)]);
    assert_eq!(headers.len(), 3);
    assert!(!headers.is_empty());

    check_header(&headers, &HeaderName::ALLOW, ALLOW, "GET");
    #[rustfmt::skip]
    check_header(&headers, &HeaderName::CONTENT_LENGTH, CONTENT_LENGTH, 123usize);
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
fn headers_get_not_found() {
    let mut headers = Headers::EMPTY;
    assert!(headers.get(&HeaderName::DATE).is_none());
    assert!(headers.get_bytes(&HeaderName::DATE).is_none());
    assert_eq!(headers.get_value::<&str>(&HeaderName::DATE), Ok(None));

    headers.append(Header::new(HeaderName::ALLOW, b"GET"));
    assert!(headers.get(&HeaderName::DATE).is_none());
    assert!(headers.get_bytes(&HeaderName::DATE).is_none());
    assert_eq!(headers.get_value::<&str>(&HeaderName::DATE), Ok(None));
}

#[test]
fn clear_headers() {
    const ALLOW: &[u8] = b"GET";
    const CONTENT_LENGTH: &[u8] = b"123";

    let mut headers = Headers::EMPTY;
    headers.append(Header::new(HeaderName::ALLOW, ALLOW));
    headers.append(Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH));
    assert_eq!(headers.len(), 2);
    assert!(!headers.is_empty());

    headers.clear();
    assert_eq!(headers.len(), 0);
    assert!(headers.is_empty());
    assert!(headers.get(&HeaderName::ALLOW).is_none());
    assert!(headers.get(&HeaderName::CONTENT_LENGTH).is_none());
}

fn check_header<'a, T>(
    headers: &'a Headers,
    name: &'_ HeaderName<'_>,
    value: &'_ [u8],
    parsed_value: T,
) where
    T: FromHeaderValue<'a> + PartialEq + fmt::Debug,
    <T as FromHeaderValue<'a>>::Err: fmt::Debug,
{
    let got = headers.get(name).unwrap();
    assert_eq!(got.name(), name);
    assert_eq!(got.value(), value);
    assert_eq!(got.parse::<T>().unwrap(), parsed_value);

    assert_eq!(headers.get_bytes(name).unwrap(), value);
    assert_eq!(headers.get_value(name).unwrap(), Some(parsed_value));
}

fn check_iter(headers: &'_ Headers, expected: &[(HeaderName<'_>, &'_ [u8])]) {
    // headers.iter.
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

    // headers.names.
    let mut len = expected.len();
    let mut names = headers.names();
    assert_eq!(names.len(), len);
    assert_eq!(names.size_hint(), (len, Some(len)));
    for (name, _) in expected {
        let got = names.next().unwrap();
        assert_eq!(got, *name);
        len -= 1;
        assert_eq!(names.len(), len);
        assert_eq!(names.size_hint(), (len, Some(len)));
    }
    assert_eq!(names.count(), 0);
    let names = headers.names();
    assert_eq!(names.count(), expected.len());

    // headers.get_all.
    for (name, value) in expected {
        let mut got = headers.get_all(name);
        let got1 = got.next().unwrap();
        assert_eq!(got1.name(), name);
        assert_eq!(got1.value(), *value);
        assert!(got.next().is_none());
    }
}

#[test]
fn headers_get_all() {
    const VALUE1: &[u8] = b"GET";
    const VALUE2: &[u8] = b"GET";
    const NAME: HeaderName<'static> = HeaderName::ALLOW;

    let mut headers = Headers::EMPTY;

    headers.append(Header::new(NAME, VALUE1));
    assert_eq!(headers.get_bytes(&NAME), Some(VALUE1));
    assert_eq!(
        headers.get_all(&NAME).collect::<Vec<_>>(),
        [Header::new(NAME, VALUE1)],
    );

    // Append with the same name.
    headers.append(Header::new(NAME, VALUE2));
    assert_eq!(headers.get_bytes(&NAME), Some(VALUE1));
    assert_eq!(
        headers.get_all(&NAME).collect::<Vec<_>>(),
        [Header::new(NAME, VALUE1), Header::new(NAME, VALUE2)],
    );
}

#[test]
fn headers_remove() {
    const VALUE1: &[u8] = b"GET";
    const VALUE2: &[u8] = b"GET";
    const NAME: HeaderName<'static> = HeaderName::ALLOW;

    let mut headers = Headers::EMPTY;

    headers.append(Header::new(NAME, VALUE1));
    headers.append(Header::new(NAME, VALUE2));

    assert_eq!(headers.get_bytes(&NAME), Some(VALUE1));
    headers.remove(&NAME);
    assert_eq!(headers.get_bytes(&NAME), Some(VALUE2));
    headers.remove(&NAME);
    assert_eq!(headers.get_bytes(&NAME), None);
}

#[test]
fn headers_remove_all() {
    const VALUE1: &[u8] = b"GET";
    const VALUE2: &[u8] = b"GET";
    const NAME: HeaderName<'static> = HeaderName::ALLOW;

    let mut headers = Headers::EMPTY;

    headers.append(Header::new(NAME, VALUE1));
    headers.append(Header::new(NAME, VALUE2));
    headers.remove_all(&NAME);
    assert_eq!(headers.get_bytes(&NAME), None);
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
        "A-IM",
        "ALPN",
        "Accept",
        "Accept-Additions",
        "Accept-CH",
        "Accept-Datetime",
        "Accept-Encoding",
        "Accept-Features",
        "Accept-Language",
        "Accept-Patch",
        "Accept-Post",
        "Accept-Ranges",
        "Access-Control-Allow-Credentials",
        "Access-Control-Allow-Headers",
        "Access-Control-Allow-Methods",
        "Access-Control-Allow-Origin",
        "Access-Control-Max-Age",
        "Access-Control-Request-Headers",
        "Access-Control-Request-Method",
        "Age",
        "Allow",
        "Alt-Svc",
        "Alt-Used",
        "Alternates",
        "Apply-To-Redirect-Ref",
        "Authentication-Control",
        "Authentication-Info",
        "Authorization",
        "CDN-Cache-Control",
        "CDN-Loop",
        "Cache-Control",
        "Cal-Managed-ID",
        "CalDAV-Timezones",
        "Capsule-Protocol",
        "Cert-Not-After",
        "Cert-Not-Before",
        "Clear-Site-Data",
        "Client-Cert",
        "Client-Cert-Chain",
        "Close",
        "Connection",
        "Content-Disposition",
        "Content-ID",
        "Content-Language",
        "Content-Length",
        "Content-Location",
        "Content-Range",
        "Content-Security-Policy-Report-Only",
        "Content-Type",
        "Cookie",
        "Cross-Origin-Embedder-Policy",
        "Cross-Origin-Embedder-Policy-Report-Only",
        "Cross-Origin-Opener-Policy",
        "Cross-Origin-Opener-Policy-Report-Only",
        "Cross-Origin-Resource-Policy",
        "DASL",
        "DAV",
        "DPoP",
        "DPoP-Nonce",
        "Date",
        "Delta-Base",
        "Depth",
        "Destination",
        "Differential-ID",
        "Digest",
        "ETag",
        "Early-Data",
        "Expect",
        "Expect-CT",
        "Expires",
        "Forwarded",
        "From",
        "Hobareg",
        "Host",
        "IM",
        "If",
        "If-Match",
        "If-Modified-Since",
        "If-None-Match",
        "If-Range",
        "If-Schedule-Tag-Match",
        "If-Unmodified-Since",
        "Include-Referred-Token-Binding-ID",
        "Keep-Alive",
        "Label",
        "Last-Event-ID",
        "Last-Modified",
        "Link",
        "Location",
        "Lock-Token",
        "MIME-Version",
        "Max-Forwards",
        "Memento-Datetime",
        "Meter",
        "Negotiate",
        "OData-EntityId",
        "OData-Isolation",
        "OData-MaxVersion",
        "OData-Version",
        "OSCORE",
        "OSLC-Core-Version",
        "Optional-WWW-Authenticate",
        "Ordering-Type",
        "Origin",
        "Origin-Agent-Cluster",
        "Overwrite",
        "Ping-From",
        "Ping-To",
        "Position",
        "Prefer",
        "Preference-Applied",
        "Priority",
        "Proxy-Authenticate",
        "Proxy-Authentication-Info",
        "Proxy-Authorization",
        "Proxy-Status",
        "Public-Key-Pins",
        "Public-Key-Pins-Report-Only",
        "Range",
        "Redirect-Ref",
        "Referer",
        "Refresh",
        "Replay-Nonce",
        "Retry-After",
        "SLUG",
        "Schedule-Reply",
        "Schedule-Tag",
        "Sec-Purpose",
        "Sec-Token-Binding",
        "Sec-WebSocket-Accept",
        "Sec-WebSocket-Extensions",
        "Sec-WebSocket-Key",
        "Sec-WebSocket-Protocol",
        "Sec-WebSocket-Version",
        "Server",
        "Server-Timing",
        "Set-Cookie",
        "Status-URI",
        "Strict-Transport-Security",
        "Sunset",
        "Surrogate-Capability",
        "Surrogate-Control",
        "TCN",
        "TE",
        "TTL",
        "Timeout",
        "Topic",
        "Traceparent",
        "Tracestate",
        "Trailer",
        "Transfer-Encoding",
        "Upgrade",
        "Urgency",
        "User-Agent",
        "Variant-Vary",
        "Vary",
        "Via",
        "WWW-Authenticate",
        "Want-Digest",
        "X-Content-Type-Options",
        "X-Frame-Options",
        "X-Request-ID",
    ];
    for name in known_headers {
        let header_name = HeaderName::from_str(name);
        assert!(!header_name.is_heap_allocated(), "header: {name}");

        // Matching should be case-insensitive.
        let header_name = HeaderName::from_str(&name.to_uppercase());
        assert!(!header_name.is_heap_allocated(), "header: {name}");
    }
}

#[test]
fn from_str_unknown_header() {
    let header_name = HeaderName::from_str("EXTRA_LONG_UNKNOWN_HEADER_NAME_REALLY_LONG");
    assert!(header_name.is_heap_allocated());
    assert_eq!(header_name, "extra_long_unknown_header_name_really_long");
}

#[test]
fn from_str_custom() {
    let unknown_headers = &["my-header", "My-Header"];
    for name in unknown_headers {
        let header_name = HeaderName::from_str(name);
        assert!(header_name.is_heap_allocated(), "header: {name}");
        assert_eq!(header_name, "my-header");
        assert_eq!(header_name.as_ref(), "my-header");
    }

    let name = "bllow"; // Matches length of "Allow" header.
    let header_name = HeaderName::from_str(name);
    assert!(header_name.is_heap_allocated(), "header: {name}");
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
