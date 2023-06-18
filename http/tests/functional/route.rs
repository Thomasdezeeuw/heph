//! Tests for the [`route!`] macro.

use heph_http::body::{EmptyBody, OneshotBody};
use heph_http::{route, Headers, Method, Request, Response, Version};
use heph_rt::test::block_on_future;

async fn route<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
    route!(match request {
        GET | HEAD "/" => index,
        GET "/test1" => handlers::get,
        HEAD "/test1" => handlers::head,
        POST "/test1" => handlers::post,
        PUT "/test1" => handlers::put,
        DELETE "/test1" => handlers::delete,
        CONNECT "/test1" => handlers::connect,
        OPTIONS "/test1" => handlers::options,
        TRACE "/test1" => handlers::trace,
        PATCH "/test1" => handlers::patch,

        POST "/test2" => handlers::post,
        _ => handlers::not_found,
    })
}

async fn index<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
    assert!(matches!(request.method(), Method::Get | Method::Head));
    assert_eq!(request.path(), "/");
    Response::ok().with_body(OneshotBody::new("index"))
}

mod handlers {
    use heph_http::body::OneshotBody;
    use heph_http::{Method, Request, Response};

    pub async fn get<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Get));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("GET"))
    }

    pub async fn head<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Head));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("HEAD"))
    }

    pub async fn post<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Post));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("POST"))
    }

    pub async fn put<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Put));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("PUT"))
    }

    pub async fn delete<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Delete));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("DELETE"))
    }

    pub async fn connect<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Connect));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("CONNECT"))
    }

    pub async fn options<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Options));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("OPTIONS"))
    }

    pub async fn trace<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Trace));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("TRACE"))
    }

    pub async fn patch<B>(request: Request<B>) -> Response<OneshotBody<&'static str>> {
        assert!(matches!(request.method(), Method::Patch));
        assert_eq!(request.path(), "/test1");
        Response::ok().with_body(OneshotBody::new("PATCH"))
    }

    pub async fn not_found<B>(_: Request<B>) -> Response<OneshotBody<&'static str>> {
        Response::not_found().with_body(OneshotBody::new("not found"))
    }
}

#[test]
fn multiple_methods_same_route() {
    block_on_future(async move {
        let tests = [Request::get("/".to_owned()), Request::head("/".to_owned())];
        for test_request in tests {
            let response = route(test_request).await;
            assert_eq!(response.body().into_inner(), "index")
        }
    })
    .unwrap();
}

#[test]
fn correct_routing_based_on_method() {
    block_on_future(async move {
        let methods = [
            Method::Options,
            Method::Get,
            Method::Post,
            Method::Put,
            Method::Delete,
            Method::Head,
            Method::Trace,
            Method::Connect,
            Method::Patch,
        ];
        for method in methods {
            let request = Request::new(
                method,
                "/test1".to_string(),
                Version::Http11,
                Headers::EMPTY,
                EmptyBody,
            );
            let response = route(request).await;
            assert_eq!(response.body().into_inner(), method.as_str())
        }
    })
    .unwrap();
}

#[test]
fn not_found_fallback() {
    block_on_future(async move {
        let tests = [
            // Unknown path.
            Request::get("/unknown".to_owned()),
            // Wrong method.
            Request::get("/test2".to_owned()),
        ];
        for test_request in tests {
            let response = route(test_request).await;
            assert_eq!(response.body().into_inner(), "not found")
        }
    })
    .unwrap();
}

// TODO: test compile failure with the following errors:
// * Not a valid method.
// * Same method & path used twice (not implemented yet).
