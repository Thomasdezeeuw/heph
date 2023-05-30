use std::fmt;

/// Response Status Code.
///
/// A complete list can be found at the HTTP Status Code Registry:
/// <http://www.iana.org/assignments/http-status-codes>.
///
/// RFC 9110 section 15.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StatusCode(pub u16);

impl StatusCode {
    // 1xx range.
    /// 100 Continue.
    ///
    /// RFC 9110 section 15.2.1.
    pub const CONTINUE: StatusCode = StatusCode(100);
    /// 101 Switching Protocols.
    ///
    /// RFC 9110 section 15.2.2.
    pub const SWITCHING_PROTOCOLS: StatusCode = StatusCode(101);
    /// 102 Processing.
    ///
    /// RFC 2518 section 10.1.
    pub const PROCESSING: StatusCode = StatusCode(102);
    /// 103 Early Hints.
    ///
    /// RFC 8297 section 2.
    pub const EARLY_HINTS: StatusCode = StatusCode(103);

    // 2xx range.
    /// 200 OK.
    ///
    /// RFC 9110 section 15.3.1.
    pub const OK: StatusCode = StatusCode(200);
    /// 201 Created.
    ///
    /// RFC 9110 section 15.3.2.
    pub const CREATED: StatusCode = StatusCode(201);
    /// 202 Accepted.
    ///
    /// RFC 9110 section 15.3.3.
    pub const ACCEPTED: StatusCode = StatusCode(202);
    /// 203 Non-Authoritative Information.
    ///
    /// RFC 9110 section 15.3.4.
    pub const NON_AUTHORITATIVE_INFORMATION: StatusCode = StatusCode(203);
    /// 204 No Content.
    ///
    /// RFC 9110 section 15.3.5.
    pub const NO_CONTENT: StatusCode = StatusCode(204);
    /// 205 Reset Content.
    ///
    /// RFC 9110 section 15.3.6.
    pub const RESET_CONTENT: StatusCode = StatusCode(205);
    /// 206 Partial Content.
    ///
    /// RFC 9110 section 15.3.7.
    pub const PARTIAL_CONTENT: StatusCode = StatusCode(206);
    /// 207 Multi-Status.
    ///
    /// RFC 4918 section 11.1.
    pub const MULTI_STATUS: StatusCode = StatusCode(207);
    /// 208 Already Reported.
    ///
    /// RFC 5842 section 7.1.
    pub const ALREADY_REPORTED: StatusCode = StatusCode(208);
    /// 226 IM Used.
    ///
    /// RFC 3229 section 10.4.1.
    pub const IM_USED: StatusCode = StatusCode(226);

    // 3xx range.
    /// 300 Multiple Choices.
    ///
    /// RFC 9110 section 15.4.1.
    pub const MULTIPLE_CHOICES: StatusCode = StatusCode(300);
    /// 301 Moved Permanently.
    ///
    /// RFC 9110 section 15.4.2.
    pub const MOVED_PERMANENTLY: StatusCode = StatusCode(301);
    /// 302 Found.
    ///
    /// RFC 9110 section 15.4.3.
    pub const FOUND: StatusCode = StatusCode(302);
    /// 303 See Other.
    ///
    /// RFC 9110 section 15.4.4.
    pub const SEE_OTHER: StatusCode = StatusCode(303);
    /// 304 Not Modified.
    ///
    /// RFC 9110 section 15.4.5.
    pub const NOT_MODIFIED: StatusCode = StatusCode(304);
    /// 305 Use Proxy.
    ///
    /// RFC 9110 section 15.4.6.
    pub const USE_PROXY: StatusCode = StatusCode(305);
    // NOTE: 306 is unused, per RFC 9110 section 15.4.7.
    /// 307 Temporary Redirect.
    ///
    /// RFC 9110 section 15.4.8.
    pub const TEMPORARY_REDIRECT: StatusCode = StatusCode(307);
    /// 308 Permanent Redirect.
    ///
    /// RFC 9110 section 15.4.9.
    pub const PERMANENT_REDIRECT: StatusCode = StatusCode(308);

    // 4xx range.
    /// 400 Bad Request.
    ///
    /// RFC 9110 section 15.5.1.
    pub const BAD_REQUEST: StatusCode = StatusCode(400);
    /// 401 Unauthorized.
    ///
    /// RFC 9110 section 15.5.2.
    pub const UNAUTHORIZED: StatusCode = StatusCode(401);
    /// 402 Payment Required.
    ///
    /// RFC 9110 section 15.5.3.
    pub const PAYMENT_REQUIRED: StatusCode = StatusCode(402);
    /// 403 Forbidden.
    ///
    /// RFC 9110 section 15.5.4.
    pub const FORBIDDEN: StatusCode = StatusCode(403);
    /// 404 Not Found.
    ///
    /// RFC 9110 section 15.5.5.
    pub const NOT_FOUND: StatusCode = StatusCode(404);
    /// 405 Method Not Allowed.
    ///
    /// RFC 9110 section 15.5.6.
    pub const METHOD_NOT_ALLOWED: StatusCode = StatusCode(405);
    /// 406 Not Acceptable.
    ///
    /// RFC 9110 section 15.5.7.
    pub const NOT_ACCEPTABLE: StatusCode = StatusCode(406);
    /// 407 Proxy Authentication Required.
    ///
    /// RFC 9110 section 15.5.8.
    pub const PROXY_AUTHENTICATION_REQUIRED: StatusCode = StatusCode(407);
    /// 408 Request Timeout.
    ///
    /// RFC 9110 section 15.5.9.
    pub const REQUEST_TIMEOUT: StatusCode = StatusCode(408);
    /// 409 Conflict.
    ///
    /// RFC 9110 section 15.5.10.
    pub const CONFLICT: StatusCode = StatusCode(409);
    /// 410 Gone.
    ///
    /// RFC 9110 section 15.5.11.
    pub const GONE: StatusCode = StatusCode(410);
    /// 411 Length Required.
    ///
    /// RFC 9110 section 15.5.12.
    pub const LENGTH_REQUIRED: StatusCode = StatusCode(411);
    /// 412 Precondition Failed.
    ///
    /// RFC 9110 section 15.5.13.
    pub const PRECONDITION_FAILED: StatusCode = StatusCode(412);
    /// 413 Payload Too Large.
    ///
    /// RFC 9110 section 15.5.14.
    pub const PAYLOAD_TOO_LARGE: StatusCode = StatusCode(413);
    /// 414 URI Too Long.
    ///
    /// RFC 9110 section 15.5.15.
    pub const URI_TOO_LONG: StatusCode = StatusCode(414);
    /// 415 Unsupported Media Type.
    ///
    /// RFC 9110 section 15.5.16.
    pub const UNSUPPORTED_MEDIA_TYPE: StatusCode = StatusCode(415);
    /// 416 Range Not Satisfiable.
    ///
    /// RFC 9110 section 15.5.17.
    pub const RANGE_NOT_SATISFIABLE: StatusCode = StatusCode(416);
    /// 417 Expectation Failed.
    ///
    /// RFC 9110 section 15.5.18.
    pub const EXPECTATION_FAILED: StatusCode = StatusCode(417);
    // NOTE: 418 is unused, 419-420 are unassigned.
    /// 421 Misdirected Request.
    ///
    /// RFC 7540 section 9.1.20.
    pub const MISDIRECTED_REQUEST: StatusCode = StatusCode(421);
    /// 422 Unprocessable Entity.
    ///
    /// RFC 7540 section 9.1.21.
    pub const UNPROCESSABLE_ENTITY: StatusCode = StatusCode(422);
    /// 423 Locked.
    ///
    /// RFC 4918, section 11.3.
    pub const LOCKED: StatusCode = StatusCode(423);
    /// 424 Failed Dependency.
    ///
    /// RFC 4918, section 11.4.
    pub const FAILED_DEPENDENCY: StatusCode = StatusCode(424);
    /// 425 Too Early.
    ///
    /// RFC 8470, section 5.2.
    pub const TOO_EARLY: StatusCode = StatusCode(425);
    /// 426 Upgrade Required.
    ///
    /// RFC 9110 section 15.5.22.
    pub const UPGRADE_REQUIRED: StatusCode = StatusCode(426);
    // NOTE: 427 is unassigned.
    /// 428 Precondition Required.
    ///
    /// RFC 6585 section 3.
    pub const PRECONDITION_REQUIRED: StatusCode = StatusCode(428);
    /// 429 Too Many Requests.
    ///
    /// RFC 6585 section 4.
    pub const TOO_MANY_REQUESTS: StatusCode = StatusCode(429);
    // NOTE: 320 is unassigned.
    /// 431 Request Header Fields Too Large.
    ///
    /// RFC 6585 section 5.
    pub const REQUEST_HEADER_FIELDS_TOO_LARGE: StatusCode = StatusCode(431);
    // NOTE: 432-450 are unassigned.
    /// 451 Unavailable For Legal Reasons.
    ///
    /// RFC 7725 section 3.
    pub const UNAVAILABLE_FOR_LEGAL_REASONS: StatusCode = StatusCode(451);

    // 5xx range.
    /// 500 Internal Server Error.
    ///
    /// RFC 9110 section 15.6.1.
    pub const INTERNAL_SERVER_ERROR: StatusCode = StatusCode(500);
    /// 501 Not Implemented.
    ///
    /// RFC 9110 section 15.6.2.
    pub const NOT_IMPLEMENTED: StatusCode = StatusCode(501);
    /// 502 Bad Gateway.
    ///
    /// RFC 9110 section 15.6.3.
    pub const BAD_GATEWAY: StatusCode = StatusCode(502);
    /// 503 Service Unavailable.
    ///
    /// RFC 9110 section 15.6.4.
    pub const SERVICE_UNAVAILABLE: StatusCode = StatusCode(503);
    /// 504 Gateway Timeout.
    ///
    /// RFC 9110 section 15.6.5.
    pub const GATEWAY_TIMEOUT: StatusCode = StatusCode(504);
    /// 505 HTTP Version Not Supported.
    ///
    /// RFC 9110 section 15.6.6.
    pub const HTTP_VERSION_NOT_SUPPORTED: StatusCode = StatusCode(505);
    /// 506 Variant Also Negotiates.
    ///
    /// RFC 2295 section 8.1.
    pub const VARIANT_ALSO_NEGOTIATES: StatusCode = StatusCode(506);
    /// 507 Insufficient Storage.
    ///
    /// RFC 4918 section 11.5.
    pub const INSUFFICIENT_STORAGE: StatusCode = StatusCode(507);
    /// 508 Loop Detected.
    ///
    /// RFC 5842 section 7.2.
    pub const LOOP_DETECTED: StatusCode = StatusCode(508);
    // NOTE: 509 is unassigned.
    /// 510 Not Extended.
    ///
    /// RFC 2774 section 7.
    pub const NOT_EXTENDED: StatusCode = StatusCode(510);
    /// 511 Network Authentication Required.
    ///
    /// RFC 6585 section 6.
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

    /// Returns `false` if the status code MUST NOT include a body.
    ///
    /// This includes the entire 1xx (Informational) range, 204 (No Content),
    /// and 304 (Not Modified).
    pub const fn includes_body(self) -> bool {
        !matches!(self.0, 100..=199 | 204 | 304)
    }

    /// Returns the reason phrase for well known status codes.
    pub const fn phrase(self) -> Option<&'static str> {
        match self.0 {
            100 => Some("Continue"),
            101 => Some("Switching Protocols"),
            102 => Some("Processing"),
            103 => Some("Early Hints"),

            200 => Some("OK"),
            201 => Some("Created"),
            202 => Some("Accepted"),
            203 => Some("Non-Authoritative Information"),
            204 => Some("No Content"),
            205 => Some("Reset Content"),
            206 => Some("Partial Content"),
            207 => Some("Multi-Status"),
            208 => Some("Already Reported"),
            226 => Some("IM Used"),

            300 => Some("Multiple Choices"),
            301 => Some("Moved Permanently"),
            302 => Some("Found"),
            303 => Some("See Other"),
            304 => Some("Not Modified"),
            305 => Some("Use Proxy"),
            307 => Some("Temporary Redirect"),
            308 => Some("Permanent Redirect"),

            400 => Some("Bad Request"),
            401 => Some("Unauthorized"),
            402 => Some("Payment Required"),
            403 => Some("Forbidden"),
            404 => Some("Not Found"),
            405 => Some("Method Not Allowed"),
            406 => Some("Not Acceptable"),
            407 => Some("Proxy Authentication Required"),
            408 => Some("Request Timeout"),
            409 => Some("Conflict"),
            410 => Some("Gone"),
            411 => Some("Length Required"),
            412 => Some("Precondition Failed"),
            413 => Some("Payload Too Large"),
            414 => Some("URI Too Long"),
            415 => Some("Unsupported Media Type"),
            416 => Some("Range Not Satisfiable"),
            417 => Some("Expectation Failed"),
            421 => Some("Misdirected Request"),
            422 => Some("Unprocessable Entity"),
            423 => Some("Locked"),
            424 => Some("Failed Dependency"),
            425 => Some("Too Early"),
            426 => Some("Upgrade Required"),
            428 => Some("Precondition Required"),
            429 => Some("Too Many Requests"),
            431 => Some("Request Header Fields Too Large"),
            451 => Some("Unavailable For Legal Reasons"),

            500 => Some("Internal Server Error"),
            501 => Some("Not Implemented"),
            502 => Some("Bad Gateway"),
            503 => Some("Service Unavailable"),
            504 => Some("Gateway Timeout"),
            505 => Some("HTTP Version Not Supported"),
            506 => Some("Variant Also Negotiates"),
            507 => Some("Insufficient Storage"),
            508 => Some("Loop Detected"),
            510 => Some("Not Extended"),
            511 => Some("Network Authentication Required"),

            _ => None,
        }
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq<u16> for StatusCode {
    fn eq(&self, other: &u16) -> bool {
        self.0.eq(other)
    }
}
