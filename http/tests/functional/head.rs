use heph_http::head::{
    Header, HeaderName, Headers, Method, RequestHead, ResponseHead, StatusCode, Version,
};

use crate::assert_size;

#[test]
fn size() {
    assert_size::<RequestHead>(80);
    assert_size::<ResponseHead>(56);
}

#[test]
fn request_header() {
    let headers = Headers::EMPTY;
    let mut head = RequestHead::new(Method::Get, "/".to_string(), Version::Http10, headers);
    assert_eq!(head.header::<&str>(&HeaderName::USER_AGENT), Ok(None));

    let header = Header::new(HeaderName::USER_AGENT, b"Heph-HTTP");
    head.headers_mut().append(header);
    assert_eq!(head.header(&HeaderName::USER_AGENT), Ok(Some("Heph-HTTP")));
}

#[test]
fn response_header() {
    let headers = Headers::EMPTY;
    let mut head = ResponseHead::new(Version::Http10, StatusCode::OK, headers);
    assert_eq!(head.header::<&str>(&HeaderName::USER_AGENT), Ok(None));

    let header = Header::new(HeaderName::USER_AGENT, b"Heph-HTTP");
    head.headers_mut().append(header);
    assert_eq!(head.header(&HeaderName::USER_AGENT), Ok(Some("Heph-HTTP")));
}
