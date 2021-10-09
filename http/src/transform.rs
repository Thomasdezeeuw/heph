//! Module with types and implementations to transform requests and response to
//! and from more convenient types.
//!
//! To transform requests from one type to another the [`From`] trait is used.
//! However since this means the documentation is somewhat all over the place
//! here is a (hopefully complete) list.
//!
//! For [`Request`]:
//!  * `&Request` -> [`Method`].
//!  * `&Request` -> [`Version`].
//!  * `&Request` -> [`Method`].
//!  * `&mut Request` -> [`Headers`] (removes the headers from the request).
//!  * `&Request` -> [`Headers`] (clones the headers).
//!  * `&mut Request` -> [`Path`] (removes the path from the request).
//!  * `&Request` -> [`Path`] (clones the path).
//!  * `Request` -> [`Body`] (consumes the entire request).
//!
//! For [`Response`]:
//!  * `&Response` -> [`Version`].
//!  * `&Response` -> [`StatusCode`].
//!  * `&mut Response` -> [`Headers`] (removes the headers from the response).
//!  * `&Response` -> [`Headers`] (clones the headers).
//!  * `Response` -> [`Body`] (consumes the entire response).
//!
//! All transformations that clone the data are wrapped in the [`Cloned`] type.
//!
//! In addition to the list above the types can be combined using the tuple
//! notation. For example to get the headers and the body you can use
//! `From::<(Headers, Body)>::from(request)`. This becomes useful when using the
//! [`TransformMiddleware`] and [`Handler`]s that expect typed data, rather then
//! HTTP types.
//!
//! Finally there is the transformation of `Result<T, E>` into `Response`, iff
//! `T` and `E` can be transformed into a `Response`.

use std::future::Future;
use std::marker::PhantomData;
use std::mem::take;
use std::pin::Pin;
use std::task::{self, Poll};

use crate::handler::{Handler, Middleware};
use crate::{Headers, Method, Request, Response, StatusCode, Version};

/// Clone (part of) the request/response before transforming, leaving the
/// original unchanged.
///
/// Transformations using the type is usually more efficient then cloning the
/// request/response before transformations because this only clones the part
/// that is returned by the transformation.
#[derive(Debug)]
pub struct Cloned<T>(pub T);

impl<B> From<&Request<B>> for Method {
    fn from(request: &Request<B>) -> Method {
        request.method()
    }
}

impl<B> From<&Request<B>> for Version {
    fn from(request: &Request<B>) -> Version {
        request.version()
    }
}

impl<B> From<&Response<B>> for Version {
    fn from(response: &Response<B>) -> Version {
        response.version()
    }
}

impl<B> From<&Response<B>> for StatusCode {
    fn from(response: &Response<B>) -> StatusCode {
        response.status()
    }
}

/// This removes the headers from the request, leaving an empty list in its
/// place.
impl<B> From<&mut Request<B>> for Headers {
    fn from(request: &mut Request<B>) -> Headers {
        take(request.headers_mut())
    }
}

impl<B> From<&Request<B>> for Cloned<Headers> {
    fn from(request: &Request<B>) -> Cloned<Headers> {
        Cloned(request.headers().clone())
    }
}

/// This removes the headers from the response, leaving an empty list in its
/// place.
impl<B> From<&mut Response<B>> for Headers {
    fn from(response: &mut Response<B>) -> Headers {
        take(response.headers_mut())
    }
}

impl<B> From<&Response<B>> for Cloned<Headers> {
    fn from(response: &Response<B>) -> Cloned<Headers> {
        Cloned(response.headers().clone())
    }
}

/// Extracts a path from a [`Request`], leaving an empty path in it's place.
///
/// If the original request should not be changed use [`Cloned`]`<Path>`
/// instead.
#[derive(Debug)]
pub struct Path(pub String);

/// This removes the path from the request, leaving an empty path in its place.
impl<B> From<&mut Request<B>> for Path {
    fn from(request: &mut Request<B>) -> Path {
        Path(take(&mut request.path))
    }
}

impl<B> From<&Request<B>> for Cloned<Path> {
    fn from(request: &Request<B>) -> Cloned<Path> {
        Cloned(Path(request.path.clone()))
    }
}

/// Extract the body from a request or response.
#[derive(Debug)]
pub struct Body<B>(pub B);

impl<B> From<Request<B>> for Body<B> {
    fn from(request: Request<B>) -> Body<B> {
        Body(request.split().1)
    }
}

impl<B> From<Response<B>> for Body<B> {
    fn from(response: Response<B>) -> Body<B> {
        Body(response.split().1)
    }
}

macro_rules! impl_from_for_tuple {
    ( $($T: ident),+ ) => {
        impl<B, $($T,)+ T> From<Request<B>> for ($($T,)+ T)
        where
            $( $T: for<'a> From<&'a mut Request<B>>, )+
            T: From<Request<B>>,
        {
            #[allow(non_snake_case)] // $T is uppercase.
            fn from(mut request: Request<B>) -> ($($T,)+ T) {
                $(let $T = From::from(&mut request);)+
                let t_last = From::from(request);
                ($($T,)+ t_last)
            }
        }

        impl<B, $($T,)+ T> From<Response<B>> for ($($T,)+ T)
        where
            $( $T: for<'a> From<&'a mut Response<B>>, )+
            T: From<Response<B>>,
        {
            #[allow(non_snake_case)] // $T is uppercase.
            fn from(mut response: Response<B>) -> ($($T,)+ T) {
                $(let $T = From::from(&mut response);)+
                let t_last = From::from(response);
                ($($T,)+ t_last)
            }
        }
    };
}

impl_from_for_tuple!(T0);
impl_from_for_tuple!(T0, T1);
impl_from_for_tuple!(T0, T1, T2);
impl_from_for_tuple!(T0, T1, T2, T3);
impl_from_for_tuple!(T0, T1, T2, T3, T4);
impl_from_for_tuple!(T0, T1, T2, T3, T4, T5);
impl_from_for_tuple!(T0, T1, T2, T3, T4, T5, T6);

impl<T, E, B> From<Result<T, E>> for Response<B>
where
    Response<B>: From<T>,
    Response<B>: From<E>,
{
    fn from(result: Result<T, E>) -> Response<B> {
        match result {
            Ok(ok) => Response::from(ok),
            Err(err) => Response::from(err),
        }
    }
}

/// [`Middleware`] to transform the request and/or response types.
///
/// This type uses a lot of generic types:
///  * `H`: the [`Handler`] this middleware wraps. It expects a request of type
///    `Req` and returns a response with type `Res`.
///  * `Req`: the request type of the handler `H`.
///  * `Res`: the response type of handler `H`.
///  * `OriginalReq`: original request type passed to this middleware, gets
///    transformed into type `Req` before being passed to the handler `H`.
///  * `TransformedResp`: the transformed (from `Res`) type returned by this
///    middleware.
#[derive(Debug)]
pub struct TransformMiddleware<H, Req, Res, TransformedResp> {
    handler: H,
    _phantom: PhantomData<(Req, Res, TransformedResp)>,
}

impl<H, Req, Res, OriginalReq, TransformedResp> Handler<OriginalReq>
    for TransformMiddleware<H, Req, Res, TransformedResp>
where
    Req: From<OriginalReq>,
    TransformedResp: From<Res>,
    H: Handler<Req, Response = Res>,
{
    type Response = TransformedResp;
    type Future = TransformFuture<H::Future, Res, TransformedResp>;

    fn handle(&self, request: OriginalReq) -> Self::Future {
        TransformFuture {
            future: self.handler.handle(request.into()),
            _phantom: PhantomData,
        }
    }
}

impl<H, Req, Res, OriginalReq, TransformedResp> Middleware<H, OriginalReq>
    for TransformMiddleware<H, Req, Res, TransformedResp>
where
    Req: From<OriginalReq>,
    TransformedResp: From<Res>,
    H: Handler<Req, Response = Res>,
{
    fn wrap(handler: H) -> Self
    where
        H: Handler<Req>,
    {
        TransformMiddleware {
            handler,
            _phantom: PhantomData,
        }
    }
}

/// [`Future`] for the [`Handler`] implementation of [`TransformMiddleware`].
#[derive(Debug)]
pub struct TransformFuture<Fut, Res, TransformedResp> {
    future: Fut,
    _phantom: PhantomData<(Res, TransformedResp)>,
}

impl<Fut, Res, TransformedResp> Future for TransformFuture<Fut, Res, TransformedResp>
where
    Fut: Future<Output = Res>,
    TransformedResp: From<Res>,
{
    type Output = TransformedResp;

    fn poll(self: Pin<&mut Self>, ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
        // SAFETY: not moving the future.
        unsafe { self.map_unchecked_mut(|s| &mut s.future) }
            .poll(ctx)
            .map(From::from)
    }
}
