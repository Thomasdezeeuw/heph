//! Module with the [`Router`] type.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::handler::Handler;
use crate::str::Str;
use crate::{Method, Request, Response};

/// Type erased boxed handler.
// TODO: use a default type for Router.
pub type BoxHandler<ReqB, ResB> = Box<
    dyn Handler<
        Request<ReqB>,
        Response = Response<ResB>,
        Future = Pin<Box<dyn Future<Output = Response<ResB>>>>,
    >,
>;

/// HTTP router.
pub struct Router<H> {
    routes: [Vec<Route<H>>; METHODS],
    not_found: H,
}

struct Route<H> {
    matcher: Matcher,
    handler: H,
}

enum Matcher {
    /// Matches a plain string.
    Path(Str<'static>),
}

impl<H> Router<H> {
    /// Create a new router routing all requests to `not_found_handler` if they
    /// don't match any route added using [`Router::add_route`].
    pub fn new<H2>(not_found_handler: H2) -> Router<H>
    where
        H2: Into<H>,
    {
        Router {
            routes: [
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ],
            not_found: not_found_handler.into(),
        }
    }

    /// Add a new route.
    pub fn add_route<H2>(self, method: Method, path: &'static str, handler: H2) -> Self
    where
        H2: Into<H>,
    {
        // TODO: use `TransformMiddleware::wrap(handler).into()`.
        self._add_route(method, Str::from_str(path), handler.into())
    }

    /// Add a new route.
    ///
    /// See [`Router::add_route`] for more docs.
    pub fn add_route_string<H2>(self, method: Method, path: String, handler: H2) -> Self
    where
        H2: Into<H>,
    {
        self._add_route(method, Str::from_string(path), handler.into())
    }

    fn _add_route(mut self, method: Method, path: Str<'static>, handler: H) -> Self {
        let route = Route {
            matcher: Matcher::Path(path),
            handler,
        };
        self.routes[method_as_index(method)].push(route);
        self
    }
}

impl Matcher {
    /// Returns `true` if `path` matches.
    fn check(&self, path: &str) -> bool {
        match self {
            Matcher::Path(p) => p == path,
        }
    }
}

impl<H, B> Handler<Request<B>> for Router<H>
where
    H: Handler<Request<B>>,
{
    type Response = H::Response;
    type Future = H::Future;

    fn handle(&self, request: Request<B>) -> Self::Future {
        let path = request.path();
        for route in &self.routes[method_as_index(request.method())] {
            if route.matcher.check(&path) {
                return route.handler.handle(request);
            }
        }
        self.not_found.handle(request)
    }
}

impl<H> fmt::Debug for Router<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_list();
        for (idx, routes) in self.routes.iter().enumerate() {
            let method = index_as_method(idx);
            for route in routes {
                let _ = f.entry(&(method, &route.matcher));
            }
        }
        f.finish()
    }
}

impl fmt::Debug for Matcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Matcher::Path(path) => f.write_str(path.as_ref()),
        }
    }
}

/// Number of supported methods.
const METHODS: usize = 9;

/// Return the index for `method`.
const fn method_as_index(method: Method) -> usize {
    use Method::*;
    match method {
        Options => 0,
        Get => 1,
        Post => 2,
        Put => 3,
        Delete => 4,
        Head => 5,
        Trace => 6,
        Connect => 7,
        Patch => 8,
    }
}

/// Returns a `Method` for `index`. Inverse of `method_as_index`.
///
/// # Notes
///
/// Returns an unspecified Method if `index > METHODS`.
const fn index_as_method(index: usize) -> Method {
    use Method::*;
    match index {
        0 => Options,
        1 => Get,
        2 => Post,
        3 => Put,
        4 => Delete,
        5 => Head,
        6 => Trace,
        7 => Connect,
        _ => Patch,
    }
}
