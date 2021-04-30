use std::fmt;

/// Response Status Code.
///
/// A complete list can be found at the HTTP Status Code Registry:
/// <http://www.iana.org/assignments/http-status-codes>.
///
/// RFC 7231 section 6.
#[derive(Copy, Clone, Debug)]
pub struct StatusCode(pub u16);

impl StatusCode {
    // 1xx range.
    /// Continue.
    ///
    /// RFC 7231 section 6.2.1.
    pub const CONTINUE: StatusCode = StatusCode(100);
    /// Switching Protocols.
    ///
    /// RFC 7231 section 6.2.2.
    pub const SWITCHING_PROTOCOLS: StatusCode = StatusCode(101);
    /// Processing.
    ///
    /// RFC 2518.
    pub const PROCESSING: StatusCode = StatusCode(103);
    /// Early Hints.
    ///
    /// RFC 8297.
    pub const EARLY_HINTS: StatusCode = StatusCode(104);

    // 2xx range.
    /// OK.
    ///
    /// RFC 7231 section 6.3.1.
    pub const OK: StatusCode = StatusCode(200);
    /// Created.
    ///
    /// RFC 7231 section 6.3.2.
    pub const CREATED: StatusCode = StatusCode(201);
    /// Accepted.
    ///
    /// RFC 7231 section 6.3.3.
    pub const ACCEPTED: StatusCode = StatusCode(202);
    /// Non-Authoritative Information.
    ///
    /// RFC 7231 section 6.3.4.
    pub const NON_AUTHORITATIVE_INFORMATION: StatusCode = StatusCode(203);
    /// No Content.
    ///
    /// RFC 7231 section 6.3.5.
    pub const NO_CONTENT: StatusCode = StatusCode(204);
    /// Reset Content.
    ///
    /// RFC 7231 section 6.3.6.
    pub const RESET_CONTENT: StatusCode = StatusCode(205);
    /// Partial Content.
    ///
    /// RFC 7233 section 4.1.
    pub const PARTIAL_CONTENT: StatusCode = StatusCode(206);
    /// Multi-Status.
    ///
    /// RFC 4918.
    pub const MULTI_STATUS: StatusCode = StatusCode(207);
    /// Already Reported.
    ///
    /// RFC 5842.
    pub const ALREADY_REPORTED: StatusCode = StatusCode(208);
    /// IM Used.
    ///
    /// RFC 3229.
    pub const IM_USED: StatusCode = StatusCode(208);

    // 3xx range.
    /// Multiple Choices.
    ///
    /// RFC 7231 section 6.4.1.
    pub const MULTIPLE_CHOICES: StatusCode = StatusCode(300);
    /// Moved Permanently.
    ///
    /// RFC 7231 section 6.4.2.
    pub const MOVED_PERMANENTLY: StatusCode = StatusCode(301);
    /// Found.
    ///
    /// RFC 7231 section 6.4.3.
    pub const FOUND: StatusCode = StatusCode(302);
    /// See Other.
    ///
    /// RFC 7231 section 6.4.4.
    pub const SEE_OTHER: StatusCode = StatusCode(303);
    /// Not Modified.
    ///
    /// RFC 7232 section 4.1.
    pub const NOT_MODIFIED: StatusCode = StatusCode(304);
    // NOTE: 306 is unused, per RFC 7231 section 6.4.6.
    /// Use Proxy.
    ///
    /// RFC 7231 section 6.4.5.
    pub const USE_PROXY: StatusCode = StatusCode(305);
    /// Temporary Redirect.
    ///
    /// RFC 7231 section 6.4.7.
    pub const TEMPORARY_REDIRECT: StatusCode = StatusCode(307);
    /// Permanent Redirect.
    ///
    /// RFC 7538.
    pub const PERMANENT_REDIRECT: StatusCode = StatusCode(308);

    // 4xx range.
    /// Bad Request.
    ///
    /// RFC 7231 section 6.5.1.
    pub const BAD_REQUEST: StatusCode = StatusCode(400);
    /// Unauthorized.
    ///
    /// RFC 7235 section 3.1.
    pub const UNAUTHORIZED: StatusCode = StatusCode(401);
    /// Payment Required.
    ///
    /// RFC 7231 section 6.5.2.
    pub const PAYMENT_REQUIRED: StatusCode = StatusCode(402);
    /// Forbidden.
    ///
    /// RFC 7231 section 6.5.3.
    pub const FORBIDDEN: StatusCode = StatusCode(403);
    /// Not Found.
    ///
    /// RFC 7231 section 6.5.4.
    pub const NOT_FOUND: StatusCode = StatusCode(404);
    /// Method Not Allowed.
    ///
    /// RFC 7231 section 6.5.5.
    pub const METHOD_NOT_ALLOWED: StatusCode = StatusCode(405);
    /// Not Acceptable.
    ///
    /// RFC 7231 section 6.5.6.
    pub const NOT_ACCEPTABLE: StatusCode = StatusCode(406);
    /// Proxy Authentication Required.
    ///
    /// RFC 7235 section 3.2.
    pub const PROXY_AUTHENTICATION_REQUIRED: StatusCode = StatusCode(407);
    /// Request Timeout.
    ///
    /// RFC 7231 section 6.5.7.
    pub const REQUEST_TIMEOUT: StatusCode = StatusCode(408);
    /// Conflict.
    ///
    /// RFC 7231 section 6.5.8.
    pub const CONFLICT: StatusCode = StatusCode(409);
    /// Gone.
    ///
    /// RFC 7231 section 6.5.9.
    pub const GONE: StatusCode = StatusCode(410);
    /// Length Required.
    ///
    /// RFC 7231 section 6.5.10.
    pub const LENGTH_REQUIRED: StatusCode = StatusCode(411);
    /// Precondition Failed.
    ///
    /// RFC 7232 section 4.2 and RFC 8144 section 3.2.
    pub const PRECONDITION_FAILED: StatusCode = StatusCode(412);
    /// Payload Too Large.
    ///
    /// RFC 7231 section 6.5.11.
    pub const PAYLOAD_TOO_LARGE: StatusCode = StatusCode(413);
    /// URI Too Long.
    ///
    /// RFC 7231 section 6.5.12.
    pub const URI_TOO_LONG: StatusCode = StatusCode(414);
    /// Unsupported Media Type.
    ///
    /// RFC 7231 section 6.5.13 and RFC 7694 section 3.
    pub const UNSUPPORTED_MEDIA_TYPE: StatusCode = StatusCode(415);
    /// Range Not Satisfiable.
    ///
    /// RFC 7233 section 4.4.
    pub const RANGE_NOT_SATISFIABLE: StatusCode = StatusCode(416);
    /// Expectation Failed.
    ///
    /// RFC 7231 section 6.5.14.
    pub const EXPECTATION_FAILED: StatusCode = StatusCode(417);
    // NOTE: 418-420 are unassigned.
    /// Misdirected Request.
    ///
    /// RFC 7540 section 9.1.2.
    pub const MISDIRECTED_REQUEST: StatusCode = StatusCode(421);
    /// Unprocessable Entity.
    ///
    /// RFC 4918.
    pub const UNPROCESSABLE_ENTITY: StatusCode = StatusCode(422);
    /// Locked.
    ///
    /// RFC 4918.
    pub const LOCKED: StatusCode = StatusCode(423);
    /// Failed Dependency.
    ///
    /// RFC 4918.
    pub const FAILED_DEPENDENCY: StatusCode = StatusCode(424);
    /// Too Early.
    ///
    /// RFC 8470.
    pub const TOO_EARLY: StatusCode = StatusCode(425);
    /// Upgrade Required.
    ///
    /// RFC 7231 section 6.5.15.
    pub const UPGRADE_REQUIRED: StatusCode = StatusCode(426);
    // NOTE: 427 is unassigned.
    /// Precondition Required.
    ///
    /// RFC 6585.
    pub const PRECONDITION_REQUIRED: StatusCode = StatusCode(428);
    /// Too Many Requests.
    ///
    /// RFC 6585.
    pub const TOO_MANY_REQUESTS: StatusCode = StatusCode(429);
    // NOTE: 320 is unassigned.
    /// Request Header Fields Too Large.
    ///
    /// RFC 6585.
    pub const REQUEST_HEADER_FIELDS_TOO_LARGE: StatusCode = StatusCode(431);
    // NOTE: 432-450 are unassigned.
    /// Unavailable For Legal Reasons.
    ///
    /// RFC 7725.
    pub const UNAVAILABLE_FOR_LEGAL_REASONS: StatusCode = StatusCode(451);

    // 5xx range.
    /// Internal Server Error.
    ///
    /// RFC 7231 section 6.6.1.
    pub const INTERNAL_SERVER_ERROR: StatusCode = StatusCode(500);
    /// Not Implemented.
    ///
    /// RFC 7231 section 6.6.2.
    pub const NOT_IMPLEMENTED: StatusCode = StatusCode(501);
    /// Bad Gateway.
    ///
    /// RFC 7231 section 6.6.3.
    pub const BAD_GATEWAY: StatusCode = StatusCode(502);
    /// Service Unavailable.
    ///
    /// RFC 7231 section 6.6.4.
    pub const SERVICE_UNAVAILABLE: StatusCode = StatusCode(503);
    /// Gateway Timeout.
    ///
    /// RFC 7231 section 6.6.5.
    pub const GATEWAY_TIMEOUT: StatusCode = StatusCode(504);
    /// HTTP Version Not Supported.
    ///
    /// RFC 7231 section 6.6.6.
    pub const HTTP_VERSION_NOT_SUPPORTED: StatusCode = StatusCode(505);
    /// Variant Also Negotiates.
    ///
    /// RFC 2295.
    pub const VARIANT_ALSO_NEGOTIATES: StatusCode = StatusCode(506);
    /// Insufficient Storage.
    ///
    /// RFC 4918.
    pub const INSUFFICIENT_STORAGE: StatusCode = StatusCode(507);
    /// Loop Detected.
    ///
    /// RFC 5842.
    pub const LOOP_DETECTED: StatusCode = StatusCode(508);
    // NOTE: 509 is unassigned.
    /// Not Extended.
    ///
    /// RFC 2774.
    pub const NOT_EXTENDED: StatusCode = StatusCode(510);
    /// Network Authentication Required.
    ///
    /// RFC 6585.
    pub const NETWORK_AUTHENTICATION_REQUIRED: StatusCode = StatusCode(511);

    /// Returns `true` if the status code is in 1xx range.
    pub const fn is_informational(self) -> bool {
        self.0 >= 100 && self.0 <= 199
    }

    /// Returns `true` if the status code is in 2xx range.
    pub const fn is_successful(self) -> bool {
        self.0 >= 200 && self.0 <= 299
    }

    /// Returns `true` if the status code is in 3xx range.
    pub const fn is_redirect(self) -> bool {
        self.0 >= 300 && self.0 <= 399
    }

    /// Returns `true` if the status code is in 4xx range.
    pub const fn is_client_error(self) -> bool {
        self.0 >= 400 && self.0 <= 499
    }

    /// Returns `true` if the status code is in 5xx range.
    pub const fn is_server_error(self) -> bool {
        self.0 >= 500 && self.0 <= 599
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}
