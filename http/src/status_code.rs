use std::fmt;

/// Response Status Code.
///
/// RFC 7231 section 6.
#[derive(Copy, Clone, Debug)]
pub struct StatusCode(u16);

// TODO: Refer to RFC 7231.
// TODO: add more constants.

impl StatusCode {
    /// Continue.
    ///
    /// Section 6.2.1
    pub const CONTINUE: StatusCode = StatusCode(100);

    /// Switching Protocols.
    ///
    /// Section 6.2.2
    pub const SWITCHING_PROTOCOLS: StatusCode = StatusCode(101);

    /// OK.
    ///
    /// Section 6.3.1
    pub const OK: StatusCode = StatusCode(200);

    /// Created.
    ///
    /// Section 6.3.2
    pub const CREATED: StatusCode = StatusCode(201);

    /// Accepted.
    ///
    /// Section 6.3.3
    pub const ACCEPTED: StatusCode = StatusCode(202);

    /// Non-Authoritative Information.
    ///
    /// Section 6.3.4
    pub const NON_AUTHORITATIVE_INFORMATION: StatusCode = StatusCode(203);

    /// No Content.
    ///
    /// Section 6.3.5
    pub const NO_CONTENT: StatusCode = StatusCode(204);

    /// Reset Content.
    ///
    /// Section 6.3.6
    pub const RESET_CONTENT: StatusCode = StatusCode(205);

    /* TODO.
    /// Partial Content.
    ///
    /// Section 4.1 of [RFC7233]
    pub const AA: StatusCode = StatusCode(206);
    /// Multiple Choices.
    ///
    /// Section 6.4.1
    pub const AA: StatusCode = StatusCode(300);
    /// Moved Permanently.
    ///
    /// Section 6.4.2
    pub const AA: StatusCode = StatusCode(301);
    /// Found.
    ///
    /// Section 6.4.3
    pub const AA: StatusCode = StatusCode(302);
    /// See Other.
    ///
    /// Section 6.4.4
    pub const AA: StatusCode = StatusCode(303);
    /// Not Modified.
    ///
    /// Section 4.1 of [RFC7232]
    pub const AA: StatusCode = StatusCode(304);
    /// Use Proxy.
    ///
    /// Section 6.4.5
    pub const AA: StatusCode = StatusCode(305);
    /// Temporary Redirect.
    ///
    /// Section 6.4.7
    pub const AA: StatusCode = StatusCode(307);
    */

    /// Bad Request.
    ///
    /// Section 6.5.1
    pub const BAD_REQUEST: StatusCode = StatusCode(400);

    /*
    /// Unauthorized.
    ///
    /// Section 3.1 of [RFC7235]
    pub const AA: StatusCode = StatusCode(401);
    /// Payment Required.
    ///
    /// Section 6.5.2
    pub const AA: StatusCode = StatusCode(402);
    /// Forbidden.
    ///
    /// Section 6.5.3
    pub const AA: StatusCode = StatusCode(403);
    */

    /// Not Found.
    ///
    /// Section 6.5.4
    pub const NOT_FOUND: StatusCode = StatusCode(404);

    /// Method Not Allowed.
    ///
    /// Section 6.5.5
    pub const METHOD_NOT_ALLOWED: StatusCode = StatusCode(405);

    /*
    /// Not Acceptable.
    ///
    /// Section 6.5.6
    pub const AA: StatusCode = StatusCode(406);
    /// Proxy Authentication Required.
    ///
    /// Section 3.2 of [RFC7235]
    pub const AA: StatusCode = StatusCode(407);
    /// Request Timeout.
    ///
    /// Section 6.5.7
    pub const AA: StatusCode = StatusCode(408);
    /// Conflict.
    ///
    /// Section 6.5.8
    pub const AA: StatusCode = StatusCode(409);
    /// Gone.
    ///
    /// Section 6.5.9
    pub const AA: StatusCode = StatusCode(410);
    /// Length Required.
    ///
    /// Section 6.5.10
    pub const AA: StatusCode = StatusCode(411);
    /// Precondition Failed.
    ///
    /// Section 4.2 of [RFC7232]
    pub const AA: StatusCode = StatusCode(412);
    */

    /// Payload Too Large.
    ///
    /// Section 6.5.11
    pub const PAYLOAD_TOO_LARGE: StatusCode = StatusCode(413);

    /*
    /// URI Too Long.
    ///
    /// Section 6.5.12
    pub const AA: StatusCode = StatusCode(414);
    /// Unsupported Media Type.
    ///
    /// Section 6.5.13
    pub const AA: StatusCode = StatusCode(415);
    /// Range Not Satisfiable.
    ///
    /// Section 4.4 of [RFC7233].
    pub const AA: StatusCode = StatusCode(416);
    /// Expectation Failed.
    ///
    /// Section 6.5.14
    pub const AA: StatusCode = StatusCode(417);
    /// Upgrade Required.
    ///
    /// Section 6.5.15
    pub const AA: StatusCode = StatusCode(426);
    /// Internal Server Error.
    ///
    /// Section 6.6.1
    pub const AA: StatusCode = StatusCode(500);
    */

    /// Not Implemented.
    ///
    /// Section 6.6.2
    pub const NOT_IMPLEMENTED: StatusCode = StatusCode(501);

    /*
    /// Bad Gateway.
    ///
    /// Section 6.6.3
    pub const AA: StatusCode = StatusCode(502);
    /// Service Unavailable.
    ///
    /// Section 6.6.4
    pub const AA: StatusCode = StatusCode(503);
    /// Gateway Timeout.
    ///
    /// Section 6.6.5
    pub const AA: StatusCode = StatusCode(504);
    /// HTTP Version Not Supported.
    ///
    /// Section 6.6.6
    pub const AA: StatusCode = StatusCode(505);
    */

    /// Returns `true` if the status code is in 1xx range.
    const fn is_informational(self) -> bool {
        self.0 >= 100 && self.0 < 199
    }

    /// Returns `true` if the status code is in 2xx range.
    const fn is_successful(self) -> bool {
        self.0 >= 200 && self.0 < 299
    }

    /// Returns `true` if the status code is in 3xx range.
    const fn is_redirect(self) -> bool {
        self.0 >= 300 && self.0 < 399
    }

    /// Returns `true` if the status code is in 4xx range.
    const fn is_client_error(self) -> bool {
        self.0 >= 400 && self.0 < 499
    }

    /// Returns `true` if the status code is in 5xx range.
    const fn is_server_error(self) -> bool {
        self.0 >= 500 && self.0 < 599
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}
