//! Module with HTTP header related types.

// TODO: impl for `Headers`.
// * FromIterator
// * Extend

use std::borrow::Cow;
use std::convert::AsRef;
use std::iter::FusedIterator;
use std::time::SystemTime;
use std::{fmt, str};

use httpdate::parse_http_date;

use crate::{cmp_lower_case, is_lower_case};

/// List of headers.
///
/// A complete list can be found at the "Message Headers" registry:
/// <http://www.iana.org/assignments/message-headers>
pub struct Headers {
    /// All values appended in a single allocation.
    values: Vec<u8>,
    /// All parts of the headers.
    parts: Vec<HeaderPart>,
}

struct HeaderPart {
    name: HeaderName<'static>,
    /// Indices into `Headers.values`.
    start: usize,
    end: usize,
}

impl Headers {
    /// Empty list of headers.
    pub const EMPTY: Headers = Headers {
        values: Vec::new(),
        parts: Vec::new(),
    };

    /// Creates new `Headers` from `headers`.
    ///
    /// Calls `F` for each header.
    pub(crate) fn from_httparse_headers<F, E>(
        raw_headers: &[httparse::Header<'_>],
        mut f: F,
    ) -> Result<Headers, E>
    where
        F: FnMut(&HeaderName, &[u8]) -> Result<(), E>,
    {
        let values_len = raw_headers.iter().map(|h| h.value.len()).sum();
        let mut headers = Headers {
            values: Vec::with_capacity(values_len),
            parts: Vec::with_capacity(raw_headers.len()),
        };
        for header in raw_headers {
            let name = HeaderName::from_str(header.name);
            let value = header.value;
            f(&name, value)?;
            headers._add(name, value);
        }
        Ok(headers)
    }

    /// Returns the number of headers.
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// Returns `true` if this is empty.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Clear the headers.
    ///
    /// Removes all headers from the list.
    pub fn clear(&mut self) {
        self.parts.clear();
        self.values.clear();
    }

    /// Add a new `header`.
    ///
    /// # Notes
    ///
    /// This doesn't check for duplicate headers, it just adds it to the list of
    /// headers.
    pub fn add<'v>(&mut self, header: Header<'static, 'v>) {
        self._add(header.name, header.value)
    }

    fn _add(&mut self, name: HeaderName<'static>, value: &[u8]) {
        let start = self.values.len();
        self.values.extend_from_slice(value);
        let end = self.values.len();
        self.parts.push(HeaderPart { name, start, end });
    }

    /// Get the header with `name`, if any.
    ///
    /// # Notes
    ///
    /// If all you need is the header value you can use [`Headers::get_value`].
    pub fn get<'a>(&'a self, name: &HeaderName<'_>) -> Option<Header<'a, 'a>> {
        for part in &self.parts {
            if part.name == *name {
                return Some(Header {
                    name: part.name.borrow(),
                    value: &self.values[part.start..part.end],
                });
            }
        }
        None
    }

    /// Get the header's value with `name`, if any.
    pub fn get_value<'a>(&'a self, name: &HeaderName) -> Option<&'a [u8]> {
        for part in &self.parts {
            if part.name == *name {
                return Some(&self.values[part.start..part.end]);
            }
        }
        None
    }

    // TODO: remove header?

    /// Returns an iterator over all headers.
    ///
    /// The order is unspecified.
    pub const fn iter<'a>(&'a self) -> Iter<'a> {
        Iter {
            headers: self,
            pos: 0,
        }
    }
}

impl From<Header<'static, '_>> for Headers {
    fn from(header: Header<'static, '_>) -> Headers {
        Headers {
            values: header.value.to_vec(),
            parts: vec![HeaderPart {
                name: header.name,
                start: 0,
                end: header.value.len(),
            }],
        }
    }
}

/*
/// # Notes
///
/// This clones the [`HeaderName`] in each header. For static headers, i.e. the
/// `HeaderName::*` constants, this is a cheap operation, for customer headers
/// this requires an allocation.
impl From<&'_ [Header<'_>]> for Headers {
    fn from(raw_headers: &'_ [Header<'_>]) -> Headers {
        let values_len = raw_headers.iter().map(|h| h.value.len()).sum();
        let mut headers = Headers {
            values: Vec::with_capacity(values_len),
            parts: Vec::with_capacity(raw_headers.len()),
        };
        for header in raw_headers {
            headers._add(header.name.clone(), header.value);
        }
        headers
    }
}
*/

impl fmt::Debug for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_map();
        for part in &self.parts {
            let value = &self.values[part.start..part.end];
            if let Ok(str) = std::str::from_utf8(value) {
                f.entry(&part.name, &str);
            } else {
                f.entry(&part.name, &value);
            }
        }
        f.finish()
    }
}

/// Iterator for [`Headers`], see [`Headers::iter`].
pub struct Iter<'a> {
    headers: &'a Headers,
    pos: usize,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Header<'a, 'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.headers.parts.get(self.pos).map(|part| {
            let header = Header {
                name: part.name.borrow(),
                value: &self.headers.values[part.start..part.end],
            };
            self.pos += 1;
            header
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }

    fn count(self) -> usize {
        self.len()
    }
}

impl<'a> ExactSizeIterator for Iter<'a> {
    fn len(&self) -> usize {
        self.headers.len() - self.pos
    }
}

impl<'a> FusedIterator for Iter<'a> {}

/// HTTP header.
///
/// RFC 7230 section 3.2.
#[derive(Clone)]
pub struct Header<'n, 'v> {
    name: HeaderName<'n>,
    value: &'v [u8],
}

impl<'n, 'v> Header<'n, 'v> {
    /// Create a new `Header`.
    ///
    /// # Notes
    ///
    /// `value` MUST NOT contain `\r\n`.
    pub const fn new(name: HeaderName<'n>, value: &'v [u8]) -> Header<'n, 'v> {
        debug_assert!(no_crlf(value), "header value contains CRLF ('\\r\\n')");
        Header { name, value }
    }

    /// Returns the name of the header.
    pub const fn name(&self) -> &HeaderName<'n> {
        &self.name
    }

    /// Returns the value of the header.
    pub const fn value(&self) -> &'v [u8] {
        self.value
    }

    /// Parse the value of the header using `T`'s [`FromHeaderValue`]
    /// implementation.
    pub fn parse<T>(&self) -> Result<T, T::Err>
    where
        T: FromHeaderValue<'v>,
    {
        FromHeaderValue::from_bytes(self.value)
    }
}

/// Returns `true` if `value` does not contain `\r\n`.
const fn no_crlf(value: &[u8]) -> bool {
    let mut i = 1;
    while i < value.len() {
        if value[i - 1] == b'\r' && value[i] == b'\n' {
            return false;
        }
        i += 1;
    }
    true
}

impl<'n, 'v> fmt::Debug for Header<'n, 'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_struct("Header");
        f.field("name", &self.name);
        if let Ok(str) = std::str::from_utf8(self.value) {
            f.field("value", &str);
        } else {
            f.field("value", &self.value);
        }
        f.finish()
    }
}

/// HTTP header name.
#[derive(Clone, PartialEq, Eq)]
pub struct HeaderName<'a> {
    /// The value MUST be lower case.
    inner: Cow<'a, str>,
}

/// Macro to create [`Name`] constants.
macro_rules! known_headers {
    ($(
        $length: tt: [
            $( $(#[$meta: meta])* ( $const_name: ident, $http_name: expr ) $(,)* ),+
        ],
    )+) => {
        $($(
            $( #[$meta] )*
            pub const $const_name: HeaderName<'static> = HeaderName::from_lowercase($http_name);
        )+)+

        /// Create a new HTTP header `HeaderName`.
        ///
        /// # Notes
        ///
        /// If `name` is static prefer to use [`HeaderName::from_lowercase`].
        #[allow(clippy::should_implement_trait)]
        pub fn from_str(name: &str) -> HeaderName<'static> {
            // This first matches on the length of the `name`, then does a
            // case-insensitive compare of the name with all known headers with
            // the same length, returning a static version if a match is found.
            match name.len() {
                $(
                $length => {
                    $(
                    if cmp_lower_case($http_name, name) {
                        return HeaderName::$const_name;
                    }
                    )+
                }
                )+
                _ => {}
            }
            // If it's not a known header return a custom (heap-allocated)
            // header name.
            HeaderName::from(name.to_string())
        }
    }
}

impl HeaderName<'static> {
    // NOTE: these are automatically generated by the `parse_headers.bash`
    // script.
    // NOTE: we adding here also add to the
    // `functional::header::from_str_known_headers` test.
    known_headers!(
        2: [
            #[doc = "IM.\n\nRFC 4229."]
            (IM, "im"),
            #[doc = "If.\n\nRFC 4918."]
            (IF, "if"),
            #[doc = "TE.\n\nRFC 7230 section 4.3."]
            (TE, "te"),
        ],
        3: [
            #[doc = "Age.\n\nRFC 7234 section 5.1."]
            (AGE, "age"),
            #[doc = "DAV.\n\nRFC 4918."]
            (DAV, "dav"),
            #[doc = "Ext.\n\nRFC 4229."]
            (EXT, "ext"),
            #[doc = "Man.\n\nRFC 4229."]
            (MAN, "man"),
            #[doc = "Opt.\n\nRFC 4229."]
            (OPT, "opt"),
            #[doc = "P3P.\n\nRFC 4229."]
            (P3P, "p3p"),
            #[doc = "PEP.\n\nRFC 4229."]
            (PEP, "pep"),
            #[doc = "TCN.\n\nRFC 4229."]
            (TCN, "tcn"),
            #[doc = "TTL.\n\nRFC 8030 section 5.2."]
            (TTL, "ttl"),
            #[doc = "URI.\n\nRFC 4229."]
            (URI, "uri"),
            #[doc = "Via.\n\nRFC 7230 section 5.7.1."]
            (VIA, "via"),
        ],
        4: [
            #[doc = "A-IM.\n\nRFC 4229."]
            (A_IM, "a-im"),
            #[doc = "ALPN.\n\nRFC 7639 section 2."]
            (ALPN, "alpn"),
            #[doc = "DASL.\n\nRFC 5323."]
            (DASL, "dasl"),
            #[doc = "Date.\n\nRFC 7231 section 7.1.1.2."]
            (DATE, "date"),
            #[doc = "ETag.\n\nRFC 7232 section 2.3."]
            (ETAG, "etag"),
            #[doc = "From.\n\nRFC 7231 section 5.5.1."]
            (FROM, "from"),
            #[doc = "Host.\n\nRFC 7230 section 5.4."]
            (HOST, "host"),
            #[doc = "Link.\n\nRFC 8288."]
            (LINK, "link"),
            #[doc = "Safe.\n\nRFC 4229."]
            (SAFE, "safe"),
            #[doc = "SLUG.\n\nRFC 5023."]
            (SLUG, "slug"),
            #[doc = "Vary.\n\nRFC 7231 section 7.1.4."]
            (VARY, "vary"),
            #[doc = "Cost.\n\nRFC 4229."]
            (COST, "cost"),
        ],
        5: [
            #[doc = "Allow.\n\nRFC 7231 section 7.4.1."]
            (ALLOW, "allow"),
            #[doc = "C-Ext.\n\nRFC 4229."]
            (C_EXT, "c-ext"),
            #[doc = "C-Man.\n\nRFC 4229."]
            (C_MAN, "c-man"),
            #[doc = "C-Opt.\n\nRFC 4229."]
            (C_OPT, "c-opt"),
            #[doc = "C-PEP.\n\nRFC 4229."]
            (C_PEP, "c-pep"),
            #[doc = "Close.\n\nRFC 7230 section 8.1."]
            (CLOSE, "close"),
            #[doc = "Depth.\n\nRFC 4918."]
            (DEPTH, "depth"),
            #[doc = "Label.\n\nRFC 4229."]
            (LABEL, "label"),
            #[doc = "Meter.\n\nRFC 4229."]
            (METER, "meter"),
            #[doc = "Range.\n\nRFC 7233 section 3.1."]
            (RANGE, "range"),
            #[doc = "Topic.\n\nRFC 8030 section 5.4."]
            (TOPIC, "topic"),
            #[doc = "SubOK.\n\nRFC 4229."]
            (SUBOK, "subok"),
            #[doc = "Subst.\n\nRFC 4229."]
            (SUBST, "subst"),
            #[doc = "Title.\n\nRFC 4229."]
            (TITLE, "title"),
        ],
        6: [
            #[doc = "Accept.\n\nRFC 7231 section 5.3.2."]
            (ACCEPT, "accept"),
            #[doc = "Cookie.\n\nRFC 6265."]
            (COOKIE, "cookie"),
            #[doc = "Digest.\n\nRFC 4229."]
            (DIGEST, "digest"),
            #[doc = "Expect.\n\nRFC 7231 section 5.1.1."]
            (EXPECT, "expect"),
            #[doc = "Origin.\n\nRFC 6454."]
            (ORIGIN, "origin"),
            #[doc = "OSCORE.\n\nRFC 8613 section 11.1."]
            (OSCORE, "oscore"),
            #[doc = "Pragma.\n\nRFC 7234 section 5.4."]
            (PRAGMA, "pragma"),
            #[doc = "Prefer.\n\nRFC 7240."]
            (PREFER, "prefer"),
            #[doc = "Public.\n\nRFC 4229."]
            (PUBLIC, "public"),
            #[doc = "Server.\n\nRFC 7231 section 7.4.2."]
            (SERVER, "server"),
            #[doc = "Sunset.\n\nRFC 8594."]
            (SUNSET, "sunset"),
        ],
        7: [
            #[doc = "Alt-Svc.\n\nRFC 7838."]
            (ALT_SVC, "alt-svc"),
            #[doc = "Cookie2.\n\nRFC 2965, RFC 6265."]
            (COOKIE2, "cookie2"),
            #[doc = "Expires.\n\nRFC 7234 section 5.3."]
            (EXPIRES, "expires"),
            #[doc = "Hobareg.\n\nRFC 7486 section 6.1.1."]
            (HOBAREG, "hobareg"),
            #[doc = "Referer.\n\nRFC 7231 section 5.5.2."]
            (REFERER, "referer"),
            #[doc = "Timeout.\n\nRFC 4918."]
            (TIMEOUT, "timeout"),
            #[doc = "Trailer.\n\nRFC 7230 section 4.4."]
            (TRAILER, "trailer"),
            #[doc = "Urgency.\n\nRFC 8030 section 5.3."]
            (URGENCY, "urgency"),
            #[doc = "Upgrade.\n\nRFC 7230 section 6.7."]
            (UPGRADE, "upgrade"),
            #[doc = "Warning.\n\nRFC 7234 section 5.5."]
            (WARNING, "warning"),
            #[doc = "Version.\n\nRFC 4229."]
            (VERSION, "version"),
        ],
        8: [
            #[doc = "Alt-Used.\n\nRFC 7838."]
            (ALT_USED, "alt-used"),
            #[doc = "CDN-Loop.\n\nRFC 8586."]
            (CDN_LOOP, "cdn-loop"),
            #[doc = "If-Match.\n\nRFC 7232 section 3.1."]
            (IF_MATCH, "if-match"),
            #[doc = "If-Range.\n\nRFC 7233 section 3.2."]
            (IF_RANGE, "if-range"),
            #[doc = "Location.\n\nRFC 7231 section 7.1.2."]
            (LOCATION, "location"),
            #[doc = "Pep-Info.\n\nRFC 4229."]
            (PEP_INFO, "pep-info"),
            #[doc = "Position.\n\nRFC 4229."]
            (POSITION, "position"),
            #[doc = "Protocol.\n\nRFC 4229."]
            (PROTOCOL, "protocol"),
            #[doc = "Optional.\n\nRFC 4229."]
            (OPTIONAL, "optional"),
            #[doc = "UA-Color.\n\nRFC 4229."]
            (UA_COLOR, "ua-color"),
            #[doc = "UA-Media.\n\nRFC 4229."]
            (UA_MEDIA, "ua-media"),
        ],
        9: [
            #[doc = "Accept-CH.\n\nRFC 8942 section 3.1."]
            (ACCEPT_CH, "accept-ch"),
            #[doc = "Expect-CT.\n\nRFC -ietf-httpbis-expect-ct-08."]
            (EXPECT_CT, "expect-ct"),
            #[doc = "Forwarded.\n\nRFC 7239."]
            (FORWARDED, "forwarded"),
            #[doc = "Negotiate.\n\nRFC 4229."]
            (NEGOTIATE, "negotiate"),
            #[doc = "Overwrite.\n\nRFC 4918."]
            (OVERWRITE, "overwrite"),
            #[doc = "Isolation.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ISOLATION, "isolation"),
            #[doc = "UA-Pixels.\n\nRFC 4229."]
            (UA_PIXELS, "ua-pixels"),
        ],
        10: [
            #[doc = "Alternates.\n\nRFC 4229."]
            (ALTERNATES, "alternates"),
            #[doc = "C-PEP-Info.\n\nRFC 4229."]
            (C_PEP_INFO, "c-pep-info"),
            #[doc = "Connection.\n\nRFC 7230 section 6.1."]
            (CONNECTION, "connection"),
            #[doc = "Content-ID.\n\nRFC 4229."]
            (CONTENT_ID, "content-id"),
            #[doc = "Delta-Base.\n\nRFC 4229."]
            (DELTA_BASE, "delta-base"),
            #[doc = "Early-Data.\n\nRFC 8470."]
            (EARLY_DATA, "early-data"),
            #[doc = "GetProfile.\n\nRFC 4229."]
            (GETPROFILE, "getprofile"),
            #[doc = "Keep-Alive.\n\nRFC 4229."]
            (KEEP_ALIVE, "keep-alive"),
            #[doc = "Lock-Token.\n\nRFC 4918."]
            (LOCK_TOKEN, "lock-token"),
            #[doc = "PICS-Label.\n\nRFC 4229."]
            (PICS_LABEL, "pics-label"),
            #[doc = "Set-Cookie.\n\nRFC 6265."]
            (SET_COOKIE, "set-cookie"),
            #[doc = "SetProfile.\n\nRFC 4229."]
            (SETPROFILE, "setprofile"),
            #[doc = "SoapAction.\n\nRFC 4229."]
            (SOAPACTION, "soapaction"),
            #[doc = "Status-URI.\n\nRFC 4229."]
            (STATUS_URI, "status-uri"),
            #[doc = "User-Agent.\n\nRFC 7231 section 5.5.3."]
            (USER_AGENT, "user-agent"),
            #[doc = "Compliance.\n\nRFC 4229."]
            (COMPLIANCE, "compliance"),
            #[doc = "Message-ID.\n\nRFC 4229."]
            (MESSAGE_ID, "message-id"),
            #[doc = "Tracestate.\n\n<https://www.w3.org/TR/trace-context/#tracestate-field>."]
            (TRACESTATE, "tracestate"),
        ],
        11: [
            #[doc = "Accept-Post.\n\n<https://www.w3.org/TR/ldp/>."]
            (ACCEPT_POST, "accept-post"),
            #[doc = "Content-MD5.\n\nRFC 4229."]
            (CONTENT_MD5, "content-md5"),
            #[doc = "Destination.\n\nRFC 4918."]
            (DESTINATION, "destination"),
            #[doc = "Retry-After.\n\nRFC 7231 section 7.1.3."]
            (RETRY_AFTER, "retry-after"),
            #[doc = "Set-Cookie2.\n\nRFC 2965, RFC 6265."]
            (SET_COOKIE2, "set-cookie2"),
            #[doc = "Want-Digest.\n\nRFC 4229."]
            (WANT_DIGEST, "want-digest"),
            #[doc = "Traceparent.\n\n<https://www.w3.org/TR/trace-context/#traceparent-field>."]
            (TRACEPARENT, "traceparent"),
        ],
        12: [
            #[doc = "Accept-Patch.\n\nRFC 5789."]
            (ACCEPT_PATCH, "accept-patch"),
            #[doc = "Content-Base.\n\nRFC 2068, RFC 2616."]
            (CONTENT_BASE, "content-base"),
            #[doc = "Content-Type.\n\nRFC 7231 section 3.1.1.5."]
            (CONTENT_TYPE, "content-type"),
            #[doc = "Derived-From.\n\nRFC 4229."]
            (DERIVED_FROM, "derived-from"),
            #[doc = "Max-Forwards.\n\nRFC 7231 section 5.1.2."]
            (MAX_FORWARDS, "max-forwards"),
            #[doc = "MIME-Version.\n\nRFC 7231, Appendix A.1."]
            (MIME_VERSION, "mime-version"),
            #[doc = "Redirect-Ref.\n\nRFC 4437."]
            (REDIRECT_REF, "redirect-ref"),
            #[doc = "Replay-Nonce.\n\nRFC 8555 section 6.5.1."]
            (REPLAY_NONCE, "replay-nonce"),
            #[doc = "Schedule-Tag.\n\nRFC 6638."]
            (SCHEDULE_TAG, "schedule-tag"),
            #[doc = "Variant-Vary.\n\nRFC 4229."]
            (VARIANT_VARY, "variant-vary"),
            #[doc = "Method-Check.\n\nW3C Web Application Formats Working Group."]
            (METHOD_CHECK, "method-check"),
            #[doc = "Referer-Root.\n\nW3C Web Application Formats Working Group."]
            (REFERER_ROOT, "referer-root"),
            #[doc = "X-Request-ID."]
            (X_REQUEST_ID, "x-request-id"),
        ],
        13: [
            #[doc = "Accept-Ranges.\n\nRFC 7233 section 2.3."]
            (ACCEPT_RANGES, "accept-ranges"),
            #[doc = "Authorization.\n\nRFC 7235 section 4.2."]
            (AUTHORIZATION, "authorization"),
            #[doc = "Cache-Control.\n\nRFC 7234 section 5.2."]
            (CACHE_CONTROL, "cache-control"),
            #[doc = "Content-Range.\n\nRFC 7233 section 4.2."]
            (CONTENT_RANGE, "content-range"),
            #[doc = "Default-Style.\n\nRFC 4229."]
            (DEFAULT_STYLE, "default-style"),
            #[doc = "If-None-Match.\n\nRFC 7232 section 3.2."]
            (IF_NONE_MATCH, "if-none-match"),
            #[doc = "Last-Modified.\n\nRFC 7232 section 2.2."]
            (LAST_MODIFIED, "last-modified"),
            #[doc = "OData-Version.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ODATA_VERSION, "odata-version"),
            #[doc = "Ordering-Type.\n\nRFC 4229."]
            (ORDERING_TYPE, "ordering-type"),
            #[doc = "ProfileObject.\n\nRFC 4229."]
            (PROFILEOBJECT, "profileobject"),
            #[doc = "Protocol-Info.\n\nRFC 4229."]
            (PROTOCOL_INFO, "protocol-info"),
            #[doc = "UA-Resolution.\n\nRFC 4229."]
            (UA_RESOLUTION, "ua-resolution"),
        ],
        14: [
            #[doc = "Accept-Charset.\n\nRFC 7231 section 5.3.3."]
            (ACCEPT_CHARSET, "accept-charset"),
            #[doc = "Cal-Managed-ID.\n\nRFC 8607 section 5.1."]
            (CAL_MANAGED_ID, "cal-managed-id"),
            #[doc = "Cert-Not-After.\n\nRFC 8739 section 3.3."]
            (CERT_NOT_AFTER, "cert-not-after"),
            #[doc = "Content-Length.\n\nRFC 7230 section 3.3.2."]
            (CONTENT_LENGTH, "content-length"),
            #[doc = "HTTP2-Settings.\n\nRFC 7540 section 3.2.1."]
            (HTTP2_SETTINGS, "http2-settings"),
            #[doc = "OData-EntityId.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ODATA_ENTITYID, "odata-entityid"),
            #[doc = "Protocol-Query.\n\nRFC 4229."]
            (PROTOCOL_QUERY, "protocol-query"),
            #[doc = "Proxy-Features.\n\nRFC 4229."]
            (PROXY_FEATURES, "proxy-features"),
            #[doc = "Schedule-Reply.\n\nRFC 6638."]
            (SCHEDULE_REPLY, "schedule-reply"),
            #[doc = "Access-Control.\n\nW3C Web Application Formats Working Group."]
            (ACCESS_CONTROL, "access-control"),
            #[doc = "Non-Compliance.\n\nRFC 4229."]
            (NON_COMPLIANCE, "non-compliance"),
        ],
        15: [
            #[doc = "Accept-Datetime.\n\nRFC 7089."]
            (ACCEPT_DATETIME, "accept-datetime"),
            #[doc = "Accept-Encoding.\n\nRFC 7231 section 5.3.4, RFC 7694 section 3."]
            (ACCEPT_ENCODING, "accept-encoding"),
            #[doc = "Accept-Features.\n\nRFC 4229."]
            (ACCEPT_FEATURES, "accept-features"),
            #[doc = "Accept-Language.\n\nRFC 7231 section 5.3.5."]
            (ACCEPT_LANGUAGE, "accept-language"),
            #[doc = "Cert-Not-Before.\n\nRFC 8739 section 3.3."]
            (CERT_NOT_BEFORE, "cert-not-before"),
            #[doc = "Content-Version.\n\nRFC 4229."]
            (CONTENT_VERSION, "content-version"),
            #[doc = "Differential-ID.\n\nRFC 4229."]
            (DIFFERENTIAL_ID, "differential-id"),
            #[doc = "OData-Isolation.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ODATA_ISOLATION, "odata-isolation"),
            #[doc = "Public-Key-Pins.\n\nRFC 7469."]
            (PUBLIC_KEY_PINS, "public-key-pins"),
            #[doc = "Security-Scheme.\n\nRFC 4229."]
            (SECURITY_SCHEME, "security-scheme"),
            #[doc = "X-Frame-Options.\n\nRFC 7034."]
            (X_FRAME_OPTIONS, "x-frame-options"),
            #[doc = "EDIINT-Features.\n\nRFC 6017."]
            (EDIINT_FEATURES, "ediint-features"),
            #[doc = "Resolution-Hint.\n\nRFC 4229."]
            (RESOLUTION_HINT, "resolution-hint"),
            #[doc = "UA-Windowpixels.\n\nRFC 4229."]
            (UA_WINDOWPIXELS, "ua-windowpixels"),
            #[doc = "X-Device-Accept.\n\nW3C Mobile Web Best Practices Working Group."]
            (X_DEVICE_ACCEPT, "x-device-accept"),
        ],
        16: [
            #[doc = "Accept-Additions.\n\nRFC 4229."]
            (ACCEPT_ADDITIONS, "accept-additions"),
            #[doc = "CalDAV-Timezones.\n\nRFC 7809 section 7.1."]
            (CALDAV_TIMEZONES, "caldav-timezones"),
            #[doc = "Content-Encoding.\n\nRFC 7231 section 3.1.2.2."]
            (CONTENT_ENCODING, "content-encoding"),
            #[doc = "Content-Language.\n\nRFC 7231 section 3.1.3.2."]
            (CONTENT_LANGUAGE, "content-language"),
            #[doc = "Content-Location.\n\nRFC 7231 section 3.1.4.2."]
            (CONTENT_LOCATION, "content-location"),
            #[doc = "Memento-Datetime.\n\nRFC 7089."]
            (MEMENTO_DATETIME, "memento-datetime"),
            #[doc = "OData-MaxVersion.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ODATA_MAXVERSION, "odata-maxversion"),
            #[doc = "Protocol-Request.\n\nRFC 4229."]
            (PROTOCOL_REQUEST, "protocol-request"),
            #[doc = "WWW-Authenticate.\n\nRFC 7235 section 4.1."]
            (WWW_AUTHENTICATE, "www-authenticate"),
        ],
        17: [
            #[doc = "If-Modified-Since.\n\nRFC 7232 section 3.3."]
            (IF_MODIFIED_SINCE, "if-modified-since"),
            #[doc = "Proxy-Instruction.\n\nRFC 4229."]
            (PROXY_INSTRUCTION, "proxy-instruction"),
            #[doc = "Sec-Token-Binding.\n\nRFC 8473."]
            (SEC_TOKEN_BINDING, "sec-token-binding"),
            #[doc = "Sec-WebSocket-Key.\n\nRFC 6455."]
            (SEC_WEBSOCKET_KEY, "sec-websocket-key"),
            #[doc = "Surrogate-Control.\n\nRFC 4229."]
            (SURROGATE_CONTROL, "surrogate-control"),
            #[doc = "Transfer-Encoding.\n\nRFC 7230 section 3.3.1."]
            (TRANSFER_ENCODING, "transfer-encoding"),
            #[doc = "OSLC-Core-Version.\n\nOASIS Project Specification 01, OASIS, Chet_Ensign."]
            (OSLC_CORE_VERSION, "oslc-core-version"),
            #[doc = "Resolver-Location.\n\nRFC 4229."]
            (RESOLVER_LOCATION, "resolver-location"),
        ],
        18: [
            #[doc = "Content-Style-Type.\n\nRFC 4229."]
            (CONTENT_STYLE_TYPE, "content-style-type"),
            #[doc = "Preference-Applied.\n\nRFC 7240."]
            (PREFERENCE_APPLIED, "preference-applied"),
            #[doc = "Proxy-Authenticate.\n\nRFC 7235 section 4.3."]
            (PROXY_AUTHENTICATE, "proxy-authenticate"),
        ],
        19: [
            #[doc = "Authentication-Info.\n\nRFC 7615 section 3."]
            (AUTHENTICATION_INFO, "authentication-info"),
            #[doc = "Content-Disposition.\n\nRFC 6266."]
            (CONTENT_DISPOSITION, "content-disposition"),
            #[doc = "Content-Script-Type.\n\nRFC 4229."]
            (CONTENT_SCRIPT_TYPE, "content-script-type"),
            #[doc = "If-Unmodified-Since.\n\nRFC 7232 section 3.4."]
            (IF_UNMODIFIED_SINCE, "if-unmodified-since"),
            #[doc = "Proxy-Authorization.\n\nRFC 7235 section 4.4."]
            (PROXY_AUTHORIZATION, "proxy-authorization"),
            #[doc = "AMP-Cache-Transform.\n\n<https://github.com/ampproject/amphtml/blob/master/spec/amp-cache-transform.md>."]
            (AMP_CACHE_TRANSFORM, "amp-cache-transform"),
            #[doc = "Timing-Allow-Origin.\n\n<https://www.w3.org/TR/resource-timing-1/#timing-allow-origin>."]
            (TIMING_ALLOW_ORIGIN, "timing-allow-origin"),
            #[doc = "X-Device-User-Agent.\n\nW3C Mobile Web Best Practices Working Group."]
            (X_DEVICE_USER_AGENT, "x-device-user-agent"),
        ],
        20: [
            #[doc = "Sec-WebSocket-Accept.\n\nRFC 6455."]
            (SEC_WEBSOCKET_ACCEPT, "sec-websocket-accept"),
            #[doc = "Surrogate-Capability.\n\nRFC 4229."]
            (SURROGATE_CAPABILITY, "surrogate-capability"),
            #[doc = "Method-Check-Expires.\n\nW3C Web Application Formats Working Group."]
            (METHOD_CHECK_EXPIRES, "method-check-expires"),
            #[doc = "Repeatability-Result.\n\nRepeatable Requests Version 1.0, OASIS, Chet_Ensign."]
            (REPEATABILITY_RESULT, "repeatability-result"),
        ],
        21: [
            #[doc = "Apply-To-Redirect-Ref.\n\nRFC 4437."]
            (APPLY_TO_REDIRECT_REF, "apply-to-redirect-ref"),
            #[doc = "If-Schedule-Tag-Match.\n\nRFC 6638."]
            (IF_SCHEDULE_TAG_MATCH, "if-schedule-tag-match"),
            #[doc = "Sec-WebSocket-Version.\n\nRFC 6455."]
            (SEC_WEBSOCKET_VERSION, "sec-websocket-version"),
        ],
        22: [
            #[doc = "Authentication-Control.\n\nRFC 8053 section 4."]
            (AUTHENTICATION_CONTROL, "authentication-control"),
            #[doc = "Sec-WebSocket-Protocol.\n\nRFC 6455."]
            (SEC_WEBSOCKET_PROTOCOL, "sec-websocket-protocol"),
            #[doc = "X-Content-Type-Options.\n\n<https://fetch.spec.whatwg.org/#x-content-type-options-header>."]
            (X_CONTENT_TYPE_OPTIONS, "x-content-type-options"),
            #[doc = "Access-Control-Max-Age.\n\nW3C Web Application Formats Working Group."]
            (ACCESS_CONTROL_MAX_AGE, "access-control-max-age"),
        ],
        23: [
            #[doc = "Repeatability-Client-ID.\n\nRepeatable Requests Version 1.0, OASIS, Chet_Ensign."]
            (REPEATABILITY_CLIENT_ID, "repeatability-client-id"),
            #[doc = "X-Device-Accept-Charset.\n\nW3C Mobile Web Best Practices Working Group."]
            (X_DEVICE_ACCEPT_CHARSET, "x-device-accept-charset"),
        ],
        24: [
            #[doc = "Sec-WebSocket-Extensions.\n\nRFC 6455."]
            (SEC_WEBSOCKET_EXTENSIONS, "sec-websocket-extensions"),
            #[doc = "Repeatability-First-Sent.\n\nRepeatable Requests Version 1.0, OASIS, Chet_Ensign."]
            (REPEATABILITY_FIRST_SENT, "repeatability-first-sent"),
            #[doc = "Repeatability-Request-ID.\n\nRepeatable Requests Version 1.0, OASIS, Chet_Ensign."]
            (REPEATABILITY_REQUEST_ID, "repeatability-request-id"),
            #[doc = "X-Device-Accept-Encoding.\n\nW3C Mobile Web Best Practices Working Group."]
            (X_DEVICE_ACCEPT_ENCODING, "x-device-accept-encoding"),
            #[doc = "X-Device-Accept-Language.\n\nW3C Mobile Web Best Practices Working Group."]
            (X_DEVICE_ACCEPT_LANGUAGE, "x-device-accept-language"),
        ],
        25: [
            #[doc = "Optional-WWW-Authenticate.\n\nRFC 8053 section 3."]
            (OPTIONAL_WWW_AUTHENTICATE, "optional-www-authenticate"),
            #[doc = "Proxy-Authentication-Info.\n\nRFC 7615 section 4."]
            (PROXY_AUTHENTICATION_INFO, "proxy-authentication-info"),
            #[doc = "Strict-Transport-Security.\n\nRFC 6797."]
            (STRICT_TRANSPORT_SECURITY, "strict-transport-security"),
            #[doc = "Content-Transfer-Encoding.\n\nRFC 4229."]
            (CONTENT_TRANSFER_ENCODING, "content-transfer-encoding"),
        ],
        27: [
            #[doc = "Public-Key-Pins-Report-Only.\n\nRFC 7469."]
            (PUBLIC_KEY_PINS_REPORT_ONLY, "public-key-pins-report-only"),
            #[doc = "Access-Control-Allow-Origin.\n\nW3C Web Application Formats Working Group."]
            (ACCESS_CONTROL_ALLOW_ORIGIN, "access-control-allow-origin"),
        ],
        28: [
            #[doc = "Access-Control-Allow-Headers.\n\nW3C Web Application Formats Working Group."]
            (ACCESS_CONTROL_ALLOW_HEADERS, "access-control-allow-headers"),
            #[doc = "Access-Control-Allow-Methods.\n\nW3C Web Application Formats Working Group."]
            (ACCESS_CONTROL_ALLOW_METHODS, "access-control-allow-methods"),
        ],
        29: [
            #[doc = "Access-Control-Request-Method.\n\nW3C Web Application Formats Working Group."]
            (ACCESS_CONTROL_REQUEST_METHOD, "access-control-request-method"),
        ],
        30: [
            #[doc = "Access-Control-Request-Headers.\n\nW3C Web Application Formats Working Group."]
            (ACCESS_CONTROL_REQUEST_HEADERS, "access-control-request-headers"),
        ],
        32: [
            #[doc = "Access-Control-Allow-Credentials.\n\nW3C Web Application Formats Working Group."]
            (ACCESS_CONTROL_ALLOW_CREDENTIALS, "access-control-allow-credentials"),
        ],
        33: [
            #[doc = "Include-Referred-Token-Binding-ID.\n\nRFC 8473."]
            (INCLUDE_REFERRED_TOKEN_BINDING_ID, "include-referred-token-binding-id"),
        ],
    );

    /// Create a new HTTP header `HeaderName`.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not all ASCII lowercase.
    pub const fn from_lowercase(name: &'static str) -> HeaderName<'static> {
        assert!(is_lower_case(name), "header name not lowercase");
        HeaderName {
            inner: Cow::Borrowed(name),
        }
    }

    /// Borrow the header name for a shorter lifetime.
    ///
    /// This is used in things like [`Headers::get`] and [`Iter`] for `Headers`
    /// to avoid clone heap-allocated `HeaderName`s.
    fn borrow<'b>(&'b self) -> HeaderName<'b> {
        HeaderName {
            inner: Cow::Borrowed(self.as_ref()),
        }
    }

    /// Returns `true` if `self` is heap allocated.
    ///
    /// # Notes
    ///
    /// This is only header to test [`HeaderName::from_str`], not part of the
    /// stable API.
    #[doc(hidden)]
    pub const fn is_heap_allocated(&self) -> bool {
        matches!(self.inner, Cow::Owned(_))
    }
}

impl From<String> for HeaderName<'static> {
    fn from(mut name: String) -> HeaderName<'static> {
        name.make_ascii_lowercase();
        HeaderName {
            inner: Cow::Owned(name),
        }
    }
}

impl<'a> AsRef<str> for HeaderName<'a> {
    fn as_ref(&self) -> &str {
        self.inner.as_ref()
    }
}

impl<'a> PartialEq<str> for HeaderName<'a> {
    fn eq(&self, other: &str) -> bool {
        // NOTE: `self` is always lowercase, per the comment on the `inner`
        // field.
        cmp_lower_case(self.inner.as_ref(), other)
    }
}

impl<'a> PartialEq<&'_ str> for HeaderName<'a> {
    fn eq(&self, other: &&str) -> bool {
        self.eq(*other)
    }
}

impl<'a> fmt::Debug for HeaderName<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl<'a> fmt::Display for HeaderName<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

/// Analogous trait to [`FromStr`].
///
/// The main use case for this trait in [`Header::parse`]. Because of this the
/// implementations should expect the `value`s passed to be ASCII/UTF-8, but
/// this not true in all cases.
///
/// [`FromStr`]: std::str::FromStr
pub trait FromHeaderValue<'a>: Sized {
    /// Error returned by parsing the bytes.
    type Err;

    /// Parse the `value`.
    fn from_bytes(value: &'a [u8]) -> Result<Self, Self::Err>;
}

#[derive(Debug)]
pub struct ParseIntError;

impl fmt::Display for ParseIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid integer")
    }
}

macro_rules! int_impl {
    ($( $ty: ty ),+) => {
        $(
        impl FromHeaderValue<'_> for $ty {
            type Err = ParseIntError;

            fn from_bytes(src: &[u8]) -> Result<Self, Self::Err> {
                if src.is_empty() {
                    return Err(ParseIntError);
                }

                let mut value: $ty = 0;
                for b in src.iter().copied() {
                    if (b'0'..=b'9').contains(&b) {
                        match value.checked_mul(10) {
                            Some(v) => value = v,
                            None => return Err(ParseIntError),
                        }
                        match value.checked_add((b - b'0') as $ty) {
                            Some(v) => value = v,
                            None => return Err(ParseIntError),
                        }
                    } else {
                        return Err(ParseIntError);
                    }
                }
                Ok(value)
            }
        }
        )+
    };
}

int_impl!(u8, u16, u32, u64, usize);

impl<'a> FromHeaderValue<'a> for &'a str {
    type Err = str::Utf8Error;

    fn from_bytes(value: &'a [u8]) -> Result<Self, Self::Err> {
        str::from_utf8(value)
    }
}

#[derive(Debug)]
pub struct ParseTimeError;

impl fmt::Display for ParseTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid time")
    }
}

/// Parses the value following RFC7231 section 7.1.1.1.
impl FromHeaderValue<'_> for SystemTime {
    type Err = ParseTimeError;

    fn from_bytes(value: &[u8]) -> Result<Self, Self::Err> {
        match str::from_utf8(value) {
            Ok(value) => match parse_http_date(value) {
                Ok(time) => Ok(time),
                Err(_) => Err(ParseTimeError),
            },
            Err(_) => Err(ParseTimeError),
        }
    }
}
