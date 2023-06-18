//! Tests for the transform module.

use heph_http::body::OneshotBody;
use heph_http::handler::{Handler, Middleware};
use heph_http::transform::{Body, Cloned, Path, TransformMiddleware};
use heph_http::{Header, HeaderName, Headers, Method, Request, Response, StatusCode, Version};
use heph_rt::test;

const REQ_BODY: &'static str = "test_body";
const OK_BODY: &'static str = "good";
const BAD_BODY: &'static str = "bad";
const HOST: &'static str = "localhost";

type TestBody = OneshotBody<&'static str>;

struct Error(&'static str);

impl From<Error> for Response<TestBody> {
    fn from(err: Error) -> Response<TestBody> {
        Response::bad_request().with_body(TestBody::new(err.0))
    }
}

struct Text(&'static str);

impl From<Text> for Response<TestBody> {
    fn from(txt: Text) -> Response<TestBody> {
        Response::ok().with_body(TestBody::new(txt.0))
    }
}

#[test]
fn transform_middleware() {
    // Test the type conversion in `TransformMiddleware`.

    async fn handler(
        method: Method,
        cloned_path: Cloned<Path>,
        path: Path,
        version: Version,
        cloned_headers: Cloned<Headers>,
        headers: Headers,
        cloned_body: Cloned<Body<TestBody>>,
        body: Body<TestBody>,
    ) -> Result<Text, Error> {
        assert_eq!(method, Method::Get);
        assert_eq!((cloned_path.0).0, path.0);
        assert_eq!(version, Version::Http11);
        let cloned_headers = cloned_headers.0;
        assert_eq!(cloned_headers.len(), 1);
        assert_eq!(headers.len(), 1);
        assert_eq!(
            cloned_headers
                .get_value::<&str>(&HeaderName::HOST)
                .unwrap()
                .unwrap(),
            HOST,
        );
        assert_eq!(
            headers
                .get_value::<&str>(&HeaderName::HOST)
                .unwrap()
                .unwrap(),
            HOST,
        );
        assert_eq!((cloned_body.0).0.into_inner(), REQ_BODY);
        assert_eq!(body.0.into_inner(), REQ_BODY);

        if path.0 == "/ok" {
            Ok(Text(OK_BODY))
        } else {
            Err(Error(BAD_BODY))
        }
    }

    let middleware = TransformMiddleware::wrap(handler);

    let tests = [
        ("/ok", StatusCode::OK, OK_BODY),
        ("/error", StatusCode::BAD_REQUEST, BAD_BODY),
    ];
    for (path, expected_status, expected_body) in tests {
        let request = Request::new(
            Method::Get,
            path.into(),
            Version::Http11,
            {
                let mut headers = Headers::EMPTY;
                headers.append(Header::new(HeaderName::HOST, HOST.as_bytes()));
                headers
            },
            TestBody::new(REQ_BODY),
        );
        let response: Response<TestBody> =
            test::block_on_future(middleware.handle(request)).unwrap();
        assert_eq!(response.status(), expected_status);
        assert_eq!(response.body().into_inner(), expected_body);
    }
}
