//! Module with HTTP method related types.

use std::fmt;
use std::str::FromStr;

use crate::cmp_lower_case;

/// HTTP method.
///
/// RFC 7231 section 4.
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
        match self {
            Method::Head => false,
            _ => true,
        }
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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by the [`FromStr`] implementation for [`Method`].
#[derive(Copy, Clone, Debug)]
pub struct UnknownMethod;

impl fmt::Display for UnknownMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown method")
    }
}

impl FromStr for Method {
    type Err = UnknownMethod;

    fn from_str(method: &str) -> Result<Self, Self::Err> {
        match method.len() {
            3 => {
                if cmp_lower_case("get", method) {
                    Ok(Method::Get)
                } else if cmp_lower_case("put", method) {
                    Ok(Method::Put)
                } else {
                    Err(UnknownMethod)
                }
            }
            4 => {
                if cmp_lower_case("head", method) {
                    Ok(Method::Head)
                } else if cmp_lower_case("post", method) {
                    Ok(Method::Post)
                } else {
                    Err(UnknownMethod)
                }
            }
            5 => {
                if cmp_lower_case("trace", method) {
                    Ok(Method::Trace)
                } else if cmp_lower_case("patch", method) {
                    Ok(Method::Patch)
                } else {
                    Err(UnknownMethod)
                }
            }
            6 => {
                if cmp_lower_case("delete", method) {
                    Ok(Method::Delete)
                } else {
                    Err(UnknownMethod)
                }
            }
            7 => {
                if cmp_lower_case("connect", method) {
                    Ok(Method::Connect)
                } else if cmp_lower_case("options", method) {
                    Ok(Method::Options)
                } else {
                    Err(UnknownMethod)
                }
            }
            _ => Err(UnknownMethod),
        }
    }
}
