use std::fmt;
use std::iter::FromIterator;

use heph_http::header::{FromHeaderValue, Header, HeaderName, Headers};

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
    assert!(!headers.is_empty());

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

    headers.add(Header::new(HeaderName::ALLOW, b"GET"));
    assert!(headers.get(&HeaderName::DATE).is_none());
    assert!(headers.get_bytes(&HeaderName::DATE).is_none());
}

#[test]
fn clear_headers() {
    const ALLOW: &[u8] = b"GET";
    const CONTENT_LENGTH: &[u8] = b"123";

    let mut headers = Headers::EMPTY;
    headers.add(Header::new(HeaderName::ALLOW, ALLOW));
    headers.add(Header::new(HeaderName::CONTENT_LENGTH, CONTENT_LENGTH));
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
        "A-IM",
        "ALPN",
        "AMP-Cache-Transform",
        "Accept",
        "Accept-Additions",
        "Accept-CH",
        "Accept-Charset",
        "Accept-Datetime",
        "Accept-Encoding",
        "Accept-Features",
        "Accept-Language",
        "Accept-Patch",
        "Accept-Post",
        "Accept-Ranges",
        "Access-Control",
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
        "C-Ext",
        "C-Man",
        "C-Opt",
        "C-PEP",
        "C-PEP-Info",
        "CDN-Loop",
        "Cache-Control",
        "Cal-Managed-ID",
        "CalDAV-Timezones",
        "Cert-Not-After",
        "Cert-Not-Before",
        "Close",
        "Compliance",
        "Connection",
        "Content-Base",
        "Content-Disposition",
        "Content-Encoding",
        "Content-ID",
        "Content-Language",
        "Content-Length",
        "Content-Location",
        "Content-MD5",
        "Content-Range",
        "Content-Script-Type",
        "Content-Style-Type",
        "Content-Transfer-Encoding",
        "Content-Type",
        "Content-Version",
        "Cookie",
        "Cookie2",
        "Cost",
        "DASL",
        "DAV",
        "Date",
        "Default-Style",
        "Delta-Base",
        "Depth",
        "Derived-From",
        "Destination",
        "Differential-ID",
        "Digest",
        "EDIINT-Features",
        "ETag",
        "Early-Data",
        "Expect",
        "Expect-CT",
        "Expires",
        "Ext",
        "Forwarded",
        "From",
        "GetProfile",
        "HTTP2-Settings",
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
        "Isolation",
        "Keep-Alive",
        "Label",
        "Last-Modified",
        "Link",
        "Location",
        "Lock-Token",
        "MIME-Version",
        "Man",
        "Max-Forwards",
        "Memento-Datetime",
        "Message-ID",
        "Meter",
        "Method-Check",
        "Method-Check-Expires",
        "Negotiate",
        "Non-Compliance",
        "OData-EntityId",
        "OData-Isolation",
        "OData-MaxVersion",
        "OData-Version",
        "OSCORE",
        "OSLC-Core-Version",
        "Opt",
        "Optional",
        "Optional-WWW-Authenticate",
        "Ordering-Type",
        "Origin",
        "Overwrite",
        "P3P",
        "PEP",
        "PICS-Label",
        "Pep-Info",
        "Position",
        "Pragma",
        "Prefer",
        "Preference-Applied",
        "ProfileObject",
        "Protocol",
        "Protocol-Info",
        "Protocol-Query",
        "Protocol-Request",
        "Proxy-Authenticate",
        "Proxy-Authentication-Info",
        "Proxy-Authorization",
        "Proxy-Features",
        "Proxy-Instruction",
        "Public",
        "Public-Key-Pins",
        "Public-Key-Pins-Report-Only",
        "Range",
        "Redirect-Ref",
        "Referer",
        "Referer-Root",
        "Repeatability-Client-ID",
        "Repeatability-First-Sent",
        "Repeatability-Request-ID",
        "Repeatability-Result",
        "Replay-Nonce",
        "Resolution-Hint",
        "Resolver-Location",
        "Retry-After",
        "SLUG",
        "Safe",
        "Schedule-Reply",
        "Schedule-Tag",
        "Sec-Token-Binding",
        "Sec-WebSocket-Accept",
        "Sec-WebSocket-Extensions",
        "Sec-WebSocket-Key",
        "Sec-WebSocket-Protocol",
        "Sec-WebSocket-Version",
        "Security-Scheme",
        "Server",
        "Set-Cookie",
        "Set-Cookie2",
        "SetProfile",
        "SoapAction",
        "Status-URI",
        "Strict-Transport-Security",
        "SubOK",
        "Subst",
        "Sunset",
        "Surrogate-Capability",
        "Surrogate-Control",
        "TCN",
        "TE",
        "TTL",
        "Timeout",
        "Timing-Allow-Origin",
        "Title",
        "Topic",
        "Traceparent",
        "Tracestate",
        "Trailer",
        "Transfer-Encoding",
        "UA-Color",
        "UA-Media",
        "UA-Pixels",
        "UA-Resolution",
        "UA-Windowpixels",
        "URI",
        "Upgrade",
        "Urgency",
        "User-Agent",
        "Variant-Vary",
        "Vary",
        "Version",
        "Via",
        "WWW-Authenticate",
        "Want-Digest",
        "Warning",
        "X-Content-Type-Options",
        "X-Device-Accept",
        "X-Device-Accept-Charset",
        "X-Device-Accept-Encoding",
        "X-Device-Accept-Language",
        "X-Device-User-Agent",
        "X-Frame-Options",
        "X-Request-ID",
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
