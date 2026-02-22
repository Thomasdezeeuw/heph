//! Types for the [`Server`] using handlers.
//!
//! See [`Server::new_using_handler`].
//!
//! [`Server`]: crate::Server
//! [`Server::new_using_handler`]: crate::Server::new_using_handler

use std::future::{self, Future};
use std::io;
use std::marker::PhantomData;

use heph::actor::{self, NewActor};
use heph::supervisor::{Supervisor, SupervisorStrategy};
use heph_rt::access::ThreadLocal;
use heph_rt::fd::AsyncFd;

use crate::body::OneshotBody;
use crate::server::{Body, Connection, RequestError};
use crate::{Header, HeaderName, Request, Response, set_nodelay};

/// [`Supervisor`] for [`Handler`].
#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct HandlerSupervisor;

impl<H, E, RT> Supervisor<Handler<H, E, RT>> for HandlerSupervisor
where
    H: HttpHandle + Clone,
    E: HttpHandleError + Clone,
{
    fn decide(&mut self, err: io::Error) -> SupervisorStrategy<AsyncFd> {
        log::error!("error handling connection: {err}");
        SupervisorStrategy::Stop
    }

    fn decide_on_restart_error(&mut self, err: !) -> SupervisorStrategy<AsyncFd> {
        err // This can't be called.
    }

    fn second_restart_error(&mut self, err: !) {
        err // This can't be called.
    }
}

/// Handler for incoming connections.
///
/// See [`Server::new_using_handler`].
///
/// [`Server::new_using_handler`]: crate::Server::new_using_handler
#[derive(Debug)]
pub struct Handler<H, E, RT = ThreadLocal> {
    handler: H,
    error_handler: E,
    // Needed for NewActor implementation.
    _phantom: PhantomData<fn(RT) -> RT>,
}

impl<H, E, RT> Handler<H, E, RT> {
    pub(crate) const fn new(handler: H, error_handler: E) -> Handler<H, E, RT> {
        Handler {
            handler,
            error_handler,
            _phantom: PhantomData,
        }
    }

    #[rustfmt::skip]
    pub(crate) fn with_error_handler<E2>(self, error_handler: E2) -> Handler<H, E2, RT> {
        let Handler { handler, .. } = self;
        Handler { handler, error_handler, _phantom: PhantomData }
    }
}

impl<H: Clone, E: Clone, RT> Clone for Handler<H, E, RT> {
    fn clone(&self) -> Handler<H, E, RT> {
        Handler {
            handler: self.handler.clone(),
            error_handler: self.error_handler.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<H, E, RT> NewActor for Handler<H, E, RT>
where
    H: HttpHandle + Clone,
    E: HttpHandleError + Clone,
{
    type Message = !;
    type Argument = AsyncFd;
    type Actor = impl Future<Output = io::Result<()>>;
    type Error = !;
    type RuntimeAccess = RT;

    fn new(
        &mut self,
        ctx: actor::Context<Self::Message, Self::RuntimeAccess>,
        fd: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        if let Some(fd) = fd.as_fd() {
            if let Err(err) = set_nodelay(fd) {
                log::warn!("failed to set TCP no delay on accepted connection: {err}");
            }
        }

        let conn = Connection::new(fd);
        Ok(http_actor(
            ctx,
            conn,
            self.handler.clone(),
            self.error_handler.clone(),
        ))
    }

    fn name() -> &'static str {
        "http_actor"
    }
}

async fn http_actor<H: HttpHandle, E: HttpHandleError, RT>(
    _ctx: actor::Context<!, RT>,
    mut conn: Connection,
    mut handler: H,
    mut error_handler: E,
) -> io::Result<()> {
    loop {
        let result = match conn.next_request().await {
            Ok(Some(request)) => Ok(handler.handle(request).await),
            Ok(None) => return Ok(()),
            Err(RequestError::Io(err)) => return Err(err),
            Err(err) => Err(err),
        };

        match result {
            Ok(response) => conn.respond_with(response).await?,
            Err(err) => {
                let should_close = err.should_close();
                let response = error_handler.handle_err(err).await;
                conn.respond_with(response).await?;
                if should_close {
                    return Ok(());
                }
            }
        }
    }
}

/// Handle a [`Request`].
///
/// See [`Server::new_using_handler`].
///
/// [`Server::new_using_handler`]: crate::Server::new_using_handler
///
/// # Examples
///
/// Using an async function.
///
/// ```
/// use heph_http::{Request, Response};
/// use heph_http::body::OneshotBody;
/// use heph_http::server::{Body, RequestError};
/// use heph_http::server::handler::HttpHandle;
///
/// async fn http_handler(req: Request<Body<'_>>) -> Response<OneshotBody<&'static str>> {
///     Response::ok().with_body(OneshotBody::new("Hello, World!"))
/// }
///
/// use_handler(http_handler);
/// fn use_handler<H: HttpHandle>(handler: H) {
///     // Do stuff with the handler...
/// #   _ = handler;
/// }
/// ```
pub trait HttpHandle {
    /// Future type used.
    type Future<'a>: Future<Output = Response<Self::Body>>;
    /// Body type used.
    type Body: crate::body::Body;

    /// Handle the request.
    fn handle<'c: 'fut, 'fut>(&mut self, request: Request<Body<'c>>) -> Self::Future<'fut>;
}

impl<T, Fut, B> HttpHandle for T
where
    T: for<'c> FnMut(Request<Body<'c>>) -> Fut,
    Fut: Future<Output = Response<B>>,
    B: crate::body::Body,
{
    type Body = B;
    type Future<'a> = Fut;

    fn handle<'c: 'fut, 'fut>(&mut self, request: Request<Body<'c>>) -> Self::Future<'fut> {
        (self)(request)
    }
}

/// Handle a [`RequestError`].
///
/// See [`Server::new_using_handler`].
///
/// [`Server::new_using_handler`]: crate::Server::new_using_handler
///
/// # Examples
///
/// The simplest implementation is using an async closure.
///
/// ```
/// use heph_http::Response;
/// use heph_http::body::OneshotBody;
/// use heph_http::server::RequestError;
/// use heph_http::server::handler::HttpHandleError;
///
/// use_error_handler(|err: RequestError| async move {
///     log::error!("error reading request: {err}");
///     err.response().with_body(OneshotBody::new(err.as_str()))
/// });
///
/// fn use_error_handler<E: HttpHandleError>(error_handler: E) {
///     // Do stuff with the error handler...
/// #   _ = error_handler;
/// }
/// ```
///
/// Or using an async function.
///
/// ```
/// use heph_http::Response;
/// use heph_http::body::OneshotBody;
/// use heph_http::server::RequestError;
/// use heph_http::server::handler::HttpHandleError;
///
/// use_error_handler(error_handler);
///
/// async fn error_handler(err: RequestError) -> Response<OneshotBody<&'static str>> {
///     // Same as the closure example above.
/// #   _ = err;
/// #   todo!();
/// }
///
/// fn use_error_handler<E: HttpHandleError>(error_handler: E) {
///     // Do stuff with the error handler...
/// #   _ = error_handler;
/// }
/// ```
pub trait HttpHandleError {
    /// Future type used.
    type Future: Future<Output = Response<Self::Body>>;
    /// Body type used.
    type Body: crate::body::Body;

    /// Handle the error.
    fn handle_err(&mut self, error: RequestError) -> Self::Future;
}

impl<T, Fut, B> HttpHandleError for T
where
    T: FnMut(RequestError) -> Fut,
    Fut: Future<Output = Response<B>>,
    B: crate::body::Body,
{
    type Body = B;
    type Future = Fut;

    fn handle_err(&mut self, err: RequestError) -> Self::Future {
        (self)(err)
    }
}

/// Default error handler.
///
/// Returns the error message returned by [`RequestError::as_str`] as plain text
/// response.
#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct DefaultErrorHandler;

impl HttpHandleError for DefaultErrorHandler {
    type Body = OneshotBody<&'static str>;
    type Future = impl Future<Output = Response<Self::Body>>;

    fn handle_err(&mut self, err: RequestError) -> Self::Future {
        let mut response = err.response();
        response.headers_mut().append_header(Header::new(
            HeaderName::CONTENT_TYPE,
            b"text/plain; charset=utf-8",
        ));
        future::ready(response.with_body(OneshotBody::new(err.as_str())))
    }
}
