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
fn request_header_or() {
    let mut request = Request::new(
        Method::Get,
        "/".to_string(),
        Version::Http10,
        Headers::EMPTY,
        EmptyBody,
    );

    // Not found.
    assert_eq!(request.header_or::<usize>(&HeaderName::CONTENT_TYPE, 0), 0);
    // OK.
    request
        .headers_mut()
        .insert(Header::new(HeaderName::CONTENT_TYPE, b"123"));
    assert_eq!(
        request.header_or::<usize>(&HeaderName::CONTENT_TYPE, 0),
        123
    );
    // Invalid.
    request
        .headers_mut()
        .insert(Header::new(HeaderName::CONTENT_TYPE, b"abc"));
    assert_eq!(request.header_or::<usize>(&HeaderName::CONTENT_TYPE, 0), 0);
}

#[test]
fn request_header_or_else() {
    let mut request = Request::new(
        Method::Get,
        "/".to_string(),
        Version::Http10,
        Headers::EMPTY,
        EmptyBody,
    );

    // Not found.
    let mut called = false;
    assert_eq!(
        request.header_or_else::<_, usize>(&HeaderName::CONTENT_TYPE, || {
            called = true;
            0
        }),
        0
    );
    assert!(called);
    // OK.
    request
        .headers_mut()
        .insert(Header::new(HeaderName::CONTENT_TYPE, b"123"));
    assert_eq!(
        request.header_or_else::<_, usize>(&HeaderName::CONTENT_TYPE, || unreachable!()),
        123
    );
    // Invalid.
    request
        .headers_mut()
        .insert(Header::new(HeaderName::CONTENT_TYPE, b"abc"));
    let mut called = false;
    assert_eq!(
        request.header_or_else::<_, usize>(&HeaderName::CONTENT_TYPE, || {
            called = true;
            0
        }),
        0
    );
    assert!(called);
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
    assert_eq!(request.body().into_inner(), BODY1);
}

#[test]
fn request_builder() {
    let tests: [(fn(String) -> Request<EmptyBody>, Method); 5] = [
        (Request::get, Method::Get),
        (Request::head, Method::Head),
        (Request::post, Method::Post),
        (Request::put, Method::Put),
        (Request::delete, Method::Delete),
    ];

    for (create, expected) in tests {
        let request = create("/".to_owned()).with_body(OneshotBody::new(BODY1));
        assert_eq!(request.method(), expected);
        assert_eq!(request.path(), "/");
        assert_eq!(request.version(), Version::Http11);
        assert!(request.headers().is_empty());
        assert_eq!(request.body().into_inner(), BODY1);
    }
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
fn response_header_or() {
    let mut response = Response::new(Version::Http10, StatusCode::OK, Headers::EMPTY, EmptyBody);

    // Not found.
    assert_eq!(response.header_or::<usize>(&HeaderName::CONTENT_TYPE, 0), 0);
    // OK.
    response
        .headers_mut()
        .insert(Header::new(HeaderName::CONTENT_TYPE, b"123"));
    assert_eq!(
        response.header_or::<usize>(&HeaderName::CONTENT_TYPE, 0),
        123
    );
    // Invalid.
    response
        .headers_mut()
        .insert(Header::new(HeaderName::CONTENT_TYPE, b"abc"));
    assert_eq!(response.header_or::<usize>(&HeaderName::CONTENT_TYPE, 0), 0);
}

#[test]
fn response_header_or_else() {
    let mut response = Response::new(Version::Http10, StatusCode::OK, Headers::EMPTY, EmptyBody);

    // Not found.
    let mut called = false;
    assert_eq!(
        response.header_or_else::<_, usize>(&HeaderName::CONTENT_TYPE, || {
            called = true;
            0
        }),
        0
    );
    assert!(called);
    // OK.
    response
        .headers_mut()
        .insert(Header::new(HeaderName::CONTENT_TYPE, b"123"));
    assert_eq!(
        response.header_or_else::<_, usize>(&HeaderName::CONTENT_TYPE, || unreachable!()),
        123
    );
    // Invalid.
    response
        .headers_mut()
        .insert(Header::new(HeaderName::CONTENT_TYPE, b"abc"));
    let mut called = false;
    assert_eq!(
        response.header_or_else::<_, usize>(&HeaderName::CONTENT_TYPE, || {
            called = true;
            0
        }),
        0
    );
    assert!(called);
}

#[test]
fn response_map_body() {
    let response = Response::new(Version::Http10, StatusCode::OK, Headers::EMPTY, EmptyBody)
        .map_body(|_body: EmptyBody| OneshotBody::new(BODY1));
    assert_eq!(response.body().into_inner(), BODY1);
}

#[test]
fn response_builder() {
    let tests: [(fn() -> Response<EmptyBody>, StatusCode); 16] = [
        (Response::ok, StatusCode::OK),
        (Response::created, StatusCode::CREATED),
        (Response::no_content, StatusCode::NO_CONTENT),
        (Response::not_modified, StatusCode::NOT_MODIFIED),
        (Response::bad_request, StatusCode::BAD_REQUEST),
        (Response::unauthorized, StatusCode::UNAUTHORIZED),
        (Response::forbidden, StatusCode::FORBIDDEN),
        (Response::not_found, StatusCode::NOT_FOUND),
        (Response::method_not_allowed, StatusCode::METHOD_NOT_ALLOWED),
        (Response::gone, StatusCode::GONE),
        (Response::length_required, StatusCode::LENGTH_REQUIRED),
        (Response::server_error, StatusCode::INTERNAL_SERVER_ERROR),
        (Response::not_implemented, StatusCode::NOT_IMPLEMENTED),
        (Response::bad_gateway, StatusCode::BAD_GATEWAY),
        (Response::unavailable, StatusCode::SERVICE_UNAVAILABLE),
        (Response::gateway_timeout, StatusCode::GATEWAY_TIMEOUT),
    ];

    for (create, expected) in tests {
        let response = create().with_body(OneshotBody::new(BODY1));
        assert_eq!(response.version(), Version::Http11);
        assert_eq!(response.status(), expected);
        assert!(response.headers().is_empty());
        assert_eq!(response.body().into_inner(), BODY1);
    }

    let tests: [(fn(&str) -> Response<EmptyBody>, StatusCode); 3] = [
        (Response::moved, StatusCode::MOVED_PERMANENTLY),
        (Response::found, StatusCode::FOUND),
        (Response::see_other, StatusCode::SEE_OTHER),
    ];

    for (create, expected) in tests {
        let uri = "/other_location";
        let response = create(uri).with_body(OneshotBody::new(BODY1));
        assert_eq!(response.version(), Version::Http11);
        assert_eq!(response.status(), expected);
        assert!(!response.headers().is_empty());
        assert_eq!(response.headers().len(), 1);
        assert_eq!(
            response.headers().get_bytes(&HeaderName::LOCATION),
            Some(uri.as_bytes())
        );
        assert_eq!(response.body().into_inner(), BODY1);
    }
}
