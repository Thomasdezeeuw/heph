//! Module with the [`route!`] macro.

/// Macro to route a request to HTTP handlers.
///
/// This macro follows a match statement-like syntax, the easiest way to
/// document that is an example:
///
/// ```
/// # #![allow(dead_code)]
/// # use heph_http::{route, Request};
/// #
/// # async fn test<B>(request: Request<B>) {
/// route!(match request {
///     // Route GET and HEAD requests to / to `index`.
///     GET | HEAD "/" => index,
///     GET | HEAD "/some_route" => some_route_handler,
///     // Route POST requests to /pet to `create_pet`.
///     POST       "/pet" => create_pet,
///     // Anything that doesn't match the routes above will be routed to the
///     // `not_found` handler.
///     _ => not_found,
/// })
/// # }
/// # async fn index<B>(_: Request<B>) { }
/// # async fn some_route_handler<B>(_: Request<B>) { }
/// # async fn create_pet<B>(_: Request<B>) { }
/// # async fn not_found<B>(_: Request<B>) { }
/// ```
///
/// The example has 5 routes:
///  * Using GET or HEAD to the path `/` will route to the `index` handler (2
///    routes),
///  * GET or HEAD to `/some_route` to `some_route_handler`, and
///  * POST to `/pet` to `create_pet`.
///
/// The match statement expects two arguments in it's pattern: a method or
/// multiple methods and a path to match. The arm must be a async function that
/// accepts the `request` and returns a response.
///
/// # Types
///
/// The macro is untyped, but expects `request` to be a [`Request`] and will
/// call at least the [`method`] and [`path`] methods on it. The generic
/// parameter `B` of `Request<B>` must be the same for all handlers, in most
/// cases this will be [`server::Body`], but it could be helpful to make the
/// handler generic of the body to make testing them simpler.
///
/// As mentioned, all handlers must accept a `Request<B>`, with the same body
/// `B` for all handlers. The handlers must be async functions, i.e. functions
/// that return a `Future<Output=Response<B>>`. Here the response body `B` must
/// also be the same for all handlers.
///
/// [`Request`]: crate::Request
/// [`method`]: crate::head::RequestHead::method
/// [`path`]: crate::head::RequestHead::path
/// [`server::Body`]: crate::server::Body
#[macro_export]
macro_rules! route {
    (match $request: ident {
        $( $method: ident $( | $method2: ident )* $path: literal => $handler: path, )+
        _ => $not_found: path $(,)?
    }) => {{
        $( $crate::_route!( _check_method $method $(, $method2 )* ); )+
        let request = $request;
        match request.method() {
            $crate::Method::Get => $crate::_route!(_single Get, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            $crate::Method::Head => $crate::_route!(_single Head, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            $crate::Method::Post => $crate::_route!(_single Post, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            $crate::Method::Put => $crate::_route!(_single Put, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            $crate::Method::Delete => $crate::_route!(_single Delete, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            $crate::Method::Connect => $crate::_route!(_single Connect, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            $crate::Method::Options => $crate::_route!(_single Options, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            $crate::Method::Trace => $crate::_route!(_single Trace, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            $crate::Method::Patch => $crate::_route!(_single Patch, request,
                $( $method $(, $method2 )* $path => $handler, )+
                _ => $not_found
            ),
            _ => $not_found(request).await,
        }
    }};
}

/// Helper macro for the [`route!`] macro.
///
/// This a private macro so that we don't clutter the public documentation with
/// all private variants of the macro.
#[doc(hidden)]
#[macro_export]
#[rustfmt::skip]
macro_rules! _route {
    // Single arm in the match statement.
    (
        _single
        $match_method: ident, $request: ident,
        $( $( $method: ident),+ $path: literal => $handler: path, )+
        _ => $not_found: path $(,)?
    ) => {{
        let path = $request.path();
        // This branch is never taken and will be removed by the compiler.
        if false {
            unreachable!()
        }
        $(
            // If any of the `$method`s match `$match_method` the `(false ||
            // $(...))` bit will return `true`, else to `false`. Depending on
            // that value the compile can either remove the branch (as `false &&
            // ...` is always false) or removes the the entire `(false || ...)`
            // bit and just check the path..
            else if (false $( || $crate::_route!(_method_filter $match_method $method) )+) && path == $path {
                $handler($request).await
            }
        )+
        else {
            $not_found($request).await
        }
    }};

    // Check the methods.
    ( _check_method GET $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method HEAD $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method POST $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method PUT $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method DELETE $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method CONNECT $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method OPTIONS $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method TRACE $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method PATCH $(, $method: ident )*) => {{ $crate::_route!(_check_method $( $method ),* ); }};
    ( _check_method ) => {{ /* All good. */ }};

    // Check if the method of route (left) matches the method of the route
    // (right).
    // If `$expected` == `$got` then true, else false.
    (_method_filter Get GET) => {{ true }};
    (_method_filter Head HEAD) => {{ true }};
    (_method_filter Post POST) => {{ true }};
    (_method_filter Put PUT) => {{ true }};
    (_method_filter Delete DELETE) => {{ true }};
    (_method_filter Connect CONNECT) => {{ true }};
    (_method_filter Options OPTIONS) => {{ true }};
    (_method_filter Trace TRACE) => {{ true }};
    (_method_filter Patch PATCH) => {{ true }};
    // No match.
    (_method_filter $expected: ident $got: ident) => {{ false }};
}
