//! Module with HTTP method related types.

use std::fmt;
use std::str::FromStr;

use crate::cmp_lower_case;

/// HTTP method.
///
/// RFC 7231 section 4.
#[non_exhaustive]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Method {
    /// GET method.
    ///
    /// RFC 7231 section 4.3.1.
    Get,
    /// HEAD method.
    ///
    /// RFC 7231 section 4.3.2.
    Head,
    /// POST method.
    ///
    /// RFC 7231 section 4.3.3.
    Post,
    /// PUT method.
    ///
    /// RFC 7231 section 4.3.4.
    Put,
    /// DELETE method.
    ///
    /// RFC 7231 section 4.3.5.
    Delete,
    /// CONNECT method.
    ///
    /// RFC 7231 section 4.3.6.
    Connect,
    /// OPTIONS method.
    ///
    /// RFC 7231 section 4.3.7.
    Options,
    /// TRACE method.
    ///
    /// RFC 7231 section 4.3.8.
    Trace,
    /// PATCH method.
    ///
    /// RFC 5789.
    Patch,
}

impl Method {
    /// Returns `true` if the method is safe.
    ///
    /// RFC 7321 section 4.2.1.
    pub const fn is_safe(self) -> bool {
        use Method::*;
        matches!(self, Get | Head | Options | Trace)
    }

    /// Returns `true` if the method is idempotent.
    ///
    /// RFC 7321 section 4.2.2.
    pub const fn is_idempotent(self) -> bool {
        matches!(self, Method::Put | Method::Delete) || self.is_safe()
    }

    /// Returns `false` if a response to this method MUST NOT include a body.
    ///
    /// This is only the case for the HEAD method.
    ///
    /// RFC 7230 section 3.3 and RFC 7321 section 4.3.2.
    pub const fn expects_body(self) -> bool {
        // RFC 7231 section 4.3.2:
        // > The HEAD method is identical to GET except that the server MUST NOT
        // > send a message body in the response (i.e., the response terminates
        // > at the end of the header section).
        !matches!(self, Method::Head)
    }

    /// Returns the method as string.
    pub const fn as_str(self) -> &'static str {
        use Method::*;
        match self {
            Options => "OPTIONS",
            Get => "GET",
            Post => "POST",
            Put => "PUT",
            Delete => "DELETE",
            Head => "HEAD",
            Trace => "TRACE",
            Connect => "CONNECT",
            Patch => "PATCH",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by the [`FromStr`] implementation for [`Method`].
#[derive(Copy, Clone, Debug)]
pub struct UnknownMethod;

impl fmt::Display for UnknownMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown HTTP method")
    }
}

impl FromStr for Method {
    type Err = UnknownMethod;

    fn from_str(method: &str) -> Result<Self, Self::Err> {
        match method.len() {
            3 => {
                if cmp_lower_case("get", method) {
                    return Ok(Method::Get);
                } else if cmp_lower_case("put", method) {
                    return Ok(Method::Put);
                }
            }
            4 => {
                if cmp_lower_case("head", method) {
                    return Ok(Method::Head);
                } else if cmp_lower_case("post", method) {
                    return Ok(Method::Post);
                }
            }
            5 => {
                if cmp_lower_case("trace", method) {
                    return Ok(Method::Trace);
                } else if cmp_lower_case("patch", method) {
                    return Ok(Method::Patch);
                }
            }
            6 => {
                if cmp_lower_case("delete", method) {
                    return Ok(Method::Delete);
                }
            }
            7 => {
                if cmp_lower_case("connect", method) {
                    return Ok(Method::Connect);
                } else if cmp_lower_case("options", method) {
                    return Ok(Method::Options);
                }
            }
            _ => {}
        }
        Err(UnknownMethod)
    }
}

impl PartialEq<str> for Method {
    fn eq(&self, other: &str) -> bool {
        match other.len() {
            3 => {
                if let Method::Get = self {
                    cmp_lower_case("get", other)
                } else if let Method::Put = self {
                    cmp_lower_case("put", other)
                } else {
                    false
                }
            }
            4 => {
                if let Method::Head = self {
                    cmp_lower_case("head", other)
                } else if let Method::Post = self {
                    cmp_lower_case("post", other)
                } else {
                    false
                }
            }
            5 => {
                if let Method::Trace = self {
                    cmp_lower_case("trace", other)
                } else if let Method::Patch = self {
                    cmp_lower_case("patch", other)
                } else {
                    false
                }
            }
            6 => {
                if let Method::Delete = self {
                    cmp_lower_case("delete", other)
                } else {
                    false
                }
            }
            7 => {
                if let Method::Connect = self {
                    cmp_lower_case("connect", other)
                } else if let Method::Options = self {
                    cmp_lower_case("options", other)
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}
