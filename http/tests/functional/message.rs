use heph_http::body::{EmptyBody, OneshotBody};
use heph_http::head::{
    Header, HeaderName, Headers, Method, RequestHead, ResponseHead, StatusCode, Version,
};
use heph_http::{Request, Response};

use crate::assert_size;

const BODY1: &'static [u8] = b"Hello world!";

#[test]
fn size() {
    assert_size::<RequestHead>(80);
    assert_size::<ResponseHead>(56);
}

#[test]
fn request_head() {
    let headers = Headers::EMPTY;
    let mut head = RequestHead::new(Method::Get, "/".to_string(), Version::Http10, headers);
    assert_eq!(head.header::<&str>(&HeaderName::USER_AGENT), Ok(None));

    let header = Header::new(HeaderName::USER_AGENT, b"Heph-HTTP");
    head.headers_mut().append(header);
    assert_eq!(head.header(&HeaderName::USER_AGENT), Ok(Some("Heph-HTTP")));

    *head.version_mut() = Version::Http11;
    assert_eq!(head.version(), Version::Http11);

    *head.method_mut() = Method::Post;
    assert_eq!(head.method(), Method::Post);
}

#[test]
fn request_map_body() {
    let request = Request::new(
        Method::Get,
        "/".to_string(),
        Version::Http10,
        Headers::EMPTY,
        EmptyBody,
    )
    .map_body(|_body: EmptyBody| OneshotBody::new(BODY1));
    assert_eq!(request.body(), BODY1);
}

#[test]
fn response_head() {
    let headers = Headers::EMPTY;
    let mut head = ResponseHead::new(Version::Http10, StatusCode::OK, headers);
    assert_eq!(head.header::<&str>(&HeaderName::USER_AGENT), Ok(None));

    let header = Header::new(HeaderName::USER_AGENT, b"Heph-HTTP");
    head.headers_mut().append(header);
    assert_eq!(head.header(&HeaderName::USER_AGENT), Ok(Some("Heph-HTTP")));

    *head.version_mut() = Version::Http11;
    assert_eq!(head.version(), Version::Http11);

    *head.status_mut() = StatusCode::CREATED;
    assert_eq!(head.status(), StatusCode::CREATED);
}

#[test]
fn response_map_body() {
    let response = Response::new(Version::Http10, StatusCode::OK, Headers::EMPTY, EmptyBody)
        .map_body(|_body: EmptyBody| OneshotBody::new(BODY1));
    assert_eq!(response.body(), BODY1);
}
