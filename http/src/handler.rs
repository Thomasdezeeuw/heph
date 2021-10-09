//! Module with the [`Handler`] and [`Middleware`] traits.
//!
//! [`Handler`]s are used to process a single request.
//!
//! [`Middleware`] is a `Handler` that wraps another handler to processes the
//! request. The middleware itself can transform the request and/or have side
//! effects such as logging the request.

use std::future::Future;

/// Handler is the trait that defines how a single request is handled.
///
/// It's basically an asynchronous function that accepts a request and
/// returns a response. In fact the function below is a valid `Handler`.
///
/// ```
/// # use heph_http::body::EmptyBody;
/// # use heph_http::handler::Handler;
/// # use heph_http::{Request, Response};
/// #
/// /// Echo the request body back as response.
/// async fn echo_handler<B>(request: Request<B>) -> Response<B> {
///     let (_, request_body) = request.split();
///     Response::ok().with_body(request_body)
/// }
/// #
/// # fn assert_handler<H: Handler<Req>, Req>(_: H) {}
/// # assert_handler::<_, (Request<EmptyBody>,)>(echo_handler);
/// ```
///
/// Note one awkward implementation detail is that  the example function has
/// type `Fn(Req) -> Res`, **however** type handler is of type
/// `Handler<(Req,)>`, that is uses a tuple with 1 field as request type. This
/// is required to support multiple arguments in function handlers.
pub trait Handler<Req> {
    /// Type of response the handler returns.
    type Response;
    /// [`Future`] returned by `handle`.
    type Future: Future<Output = Self::Response>;

    /// Returns a [`Future`] that handles a single `request` and returns a
    /// response.
    fn handle(&self, request: Req) -> Self::Future;
}

macro_rules! impl_handler_for_fn_tuple {
    ( $($T: ident),+ ) => {
        impl<F, $($T,)+ Res, Fut> Handler<($($T,)+)> for F
        where
            F: Fn($($T,)+) -> Fut,
            Fut: Future<Output = Res>,
        {
            type Response = Res;
            type Future = Fut;

            #[allow(non_snake_case)] // $T is uppercase.
            fn handle(&self, request: ($($T,)+)) -> Self::Future {
                let ($($T,)+) = request;
                (self)($($T,)+)
            }
        }
    };
}

impl_handler_for_fn_tuple!(Req0);
impl_handler_for_fn_tuple!(Req0, Req1);
impl_handler_for_fn_tuple!(Req0, Req1, Req2);
impl_handler_for_fn_tuple!(Req0, Req1, Req2, Req3);
impl_handler_for_fn_tuple!(Req0, Req1, Req2, Req3, Req4);
impl_handler_for_fn_tuple!(Req0, Req1, Req2, Req3, Req4, Req5);
impl_handler_for_fn_tuple!(Req0, Req1, Req2, Req3, Req4, Req5, Req6);
impl_handler_for_fn_tuple!(Req0, Req1, Req2, Req3, Req4, Req5, Req6, Req7);

/// Middleware wraps other [`Handler`]s to transform the request and/or
/// response. In addition to transforming the requests and/or response they can
/// also have side effects, e.g. logging the request.
///
/// What middleware allows to do is create composable components that can be run
/// for any route having little or no knowledge of the underlying handler. An
/// example is logging the request, it doesn't have to know anything about how
/// the request is processed, all it sees is the request and the response.
///
/// Middleware is not in any way special, they are simply [`Handler`]s, but
/// instead of creating a response themselves they call another handler to do
/// that.
pub trait Middleware<H, Req>: Handler<Req> {
    /// Wrap `handler` to create new middleware.
    fn wrap(handler: H) -> Self
    where
        H: Handler<Req>;
}
