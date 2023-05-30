//! Method related types.

use std::fmt;
use std::str::FromStr;

use crate::cmp_lower_case;

/// HTTP method.
///
/// RFC 9110 section 9.3
#[non_exhaustive]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Method {
    /// GET method.
    ///
    /// RFC 9110 section 9.3.1.
    Get,
    /// HEAD method.
    ///
    /// RFC 9110 section 9.3.2.
    Head,
    /// POST method.
    ///
    /// RFC 9110 section 9.3.3.
    Post,
    /// PUT method.
    ///
    /// RFC 9110 section 9.3.4.
    Put,
    /// DELETE method.
    ///
    /// RFC 9110 section 9.3.5.
    Delete,
    /// CONNECT method.
    ///
    /// RFC 9110 section 9.3.6.
    Connect,
    /// OPTIONS method.
    ///
    /// RFC 9110 section 9.3.7.
    Options,
    /// TRACE method.
    ///
    /// RFC 9110 section 9.3.8.
    Trace,
    /// PATCH method.
    ///
    /// RFC 5789.
    Patch,
}

impl Method {
    /// Returns `true` if the method is safe.
    ///
    /// RFC 9110 section 9.2.1.
    #[rustfmt::skip]
    pub const fn is_safe(self) -> bool {
        matches!(self, Method::Get | Method::Head | Method::Options | Method::Trace)
    }

    /// Returns `true` if the method is idempotent.
    ///
    /// RFC 9110 section 9.2.2.
    pub const fn is_idempotent(self) -> bool {
        matches!(self, Method::Put | Method::Delete) || self.is_safe()
    }

    /// Returns `false` if a response to this method MUST NOT include a body.
    ///
    /// This is only the case for the HEAD method.
    ///
    /// RFC 9110 section 6.4.1.
    pub const fn expects_body(self) -> bool {
        !matches!(self, Method::Head)
    }

    /// Returns the method as string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Method::Options => "OPTIONS",
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Head => "HEAD",
            Method::Trace => "TRACE",
            Method::Connect => "CONNECT",
            Method::Patch => "PATCH",
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
