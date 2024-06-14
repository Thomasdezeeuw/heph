//! Header related types.
//!
//! This module has three main types:
//!  * [`Headers`] is a list of mulitple headers,
//!  * [`Header`] is a single header, and finally
//!  * [`HeaderName`] is the name of a header.
//!
//! A list of known (registered) headers in available in [`HeaderName`].

use std::iter::FusedIterator;
use std::time::SystemTime;
use std::{fmt, str};

use httpdate::parse_http_date;

use crate::str::Str;
use crate::{cmp_lower_case, is_lower_case};

/// List of headers.
///
/// A complete list can be found at the HTTP Field Name Registry:
/// <https://www.iana.org/assignments/http-fields/http-fields.xhtml>.
#[derive(Clone)]
pub struct Headers {
    /// All values appended in a single allocation.
    values: Vec<u8>,
    /// All parts of the headers.
    parts: Vec<HeaderPart>,
}

#[derive(Clone)]
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
        F: FnMut(&HeaderName<'_>, &[u8]) -> Result<(), E>,
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
            headers._append(name, value);
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

    /// Append a new `header`.
    ///
    /// # Notes
    ///
    /// This doesn't check for duplicate headers, it just adds it to the list of
    /// headers. This means the list can contain two headers with the same name.
    /// If you don't want duplicate headers you can use (the more expansive)
    /// [`Headers::insert`] method.
    pub fn append(&mut self, header: Header<'static, '_>) {
        self._append(header.name, header.value);
    }

    /// Insert `header`, removing all existing headers with the same name.
    pub fn insert(&mut self, header: Header<'static, '_>) {
        self.remove_all(&header.name);
        self._append(header.name, header.value);
    }

    fn _append(&mut self, name: HeaderName<'static>, value: &[u8]) {
        let start = self.values.len();
        self.values.extend_from_slice(value);
        let end = self.values.len();
        self.parts.push(HeaderPart { name, start, end });
    }

    /// Get the first header with `name`, if any.
    ///
    /// # Notes
    ///
    /// If all you need is the header value you can use [`Headers::get_value`]
    /// or [`Headers::get_bytes`] for the raw value.
    pub fn get(&self, name: &HeaderName<'_>) -> Option<Header<'_, '_>> {
        self.parts
            .iter()
            .find(|part| part.name == *name)
            .map(|part| Header {
                name: part.name.borrow(),
                value: &self.values[part.start..part.end],
            })
    }

    /// Get all headers with `name`, if any.
    pub fn get_all<'h, 'n>(
        &'h self,
        name: &'n HeaderName<'n>,
    ) -> impl Iterator<Item = Header<'n, 'h>> {
        self.parts
            .iter()
            .filter(|part| (part.name == *name))
            .map(move |part| Header {
                name: name.borrow(),
                value: &self.values[part.start..part.end],
            })
    }

    /// Get the header's value with `name`, if any.
    ///
    /// This returns `Ok(None)` if there is no header with `name` and `Err(..)`
    /// in case [`FromHeaderValue`] for `T` returns an error.
    pub fn get_value<'a, T>(&'a self, name: &HeaderName<'_>) -> Result<Option<T>, T::Err>
    where
        T: FromHeaderValue<'a>,
    {
        match self.get_bytes(name) {
            Some(value) => FromHeaderValue::from_bytes(value).map(Some),
            None => Ok(None),
        }
    }

    /// Get the header's value with `name` as byte slice, if any.
    pub fn get_bytes<'a>(&'a self, name: &HeaderName<'_>) -> Option<&'a [u8]> {
        for part in &self.parts {
            if part.name == *name {
                return Some(&self.values[part.start..part.end]);
            }
        }
        None
    }

    /// Remove the first header with `name`.
    pub fn remove(&mut self, name: &HeaderName<'_>) {
        if let Some(idx) = self.parts.iter().position(move |part| part.name == *name) {
            drop(self.parts.remove(idx));
        }
    }

    /// Remove all headers with `name`.
    pub fn remove_all(&mut self, name: &HeaderName<'_>) {
        self.parts
            .extract_if(move |part| part.name == *name)
            .for_each(drop);
    }

    /// Returns an iterator that iterates over all headers.
    ///
    /// The order is unspecified.
    pub const fn iter<'a>(&'a self) -> Iter<'a> {
        Iter {
            headers: self,
            pos: 0,
        }
    }

    /// Returns an iterator that iterates over all header names.
    ///
    /// The order is unspecified.
    ///
    /// # Notes
    ///
    /// This may return names twice if `self` contains the name twice.
    pub const fn names<'a>(&'a self) -> Names<'a> {
        Names {
            headers: self,
            pos: 0,
        }
    }
}

impl Default for Headers {
    fn default() -> Headers {
        Headers::EMPTY
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

impl<const N: usize> From<[Header<'static, '_>; N]> for Headers {
    fn from(raw_headers: [Header<'static, '_>; N]) -> Headers {
        let values_len = raw_headers.iter().map(|h| h.value.len()).sum();
        let mut headers = Headers {
            values: Vec::with_capacity(values_len),
            parts: Vec::with_capacity(raw_headers.len()),
        };
        for header in raw_headers {
            headers._append(header.name.clone(), header.value);
        }
        headers
    }
}

impl From<&[Header<'static, '_>]> for Headers {
    fn from(raw_headers: &[Header<'static, '_>]) -> Headers {
        let values_len = raw_headers.iter().map(|h| h.value.len()).sum();
        let mut headers = Headers {
            values: Vec::with_capacity(values_len),
            parts: Vec::with_capacity(raw_headers.len()),
        };
        for header in raw_headers {
            headers._append(header.name.clone(), header.value);
        }
        headers
    }
}

impl<'v> FromIterator<Header<'static, 'v>> for Headers {
    fn from_iter<I>(iter: I) -> Headers
    where
        I: IntoIterator<Item = Header<'static, 'v>>,
    {
        let mut headers = Headers::EMPTY;
        headers.extend(iter);
        headers
    }
}

impl<'v> Extend<Header<'static, 'v>> for Headers {
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Header<'static, 'v>>,
    {
        let iter = iter.into_iter();
        let (iter_len, _) = iter.size_hint();
        // Make a guess of 10 bytes per header value on average.
        self.values.reserve(iter_len * 10);
        self.parts.reserve(iter_len);
        for header in iter {
            self._append(header.name, header.value);
        }
    }
}

impl<'a> IntoIterator for &'a Headers {
    type Item = Header<'a, 'a>;

    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl fmt::Debug for Headers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_map();
        for part in &self.parts {
            let value = &self.values[part.start..part.end];
            if let Ok(str) = str::from_utf8(value) {
                let _ = f.entry(&part.name, &str);
            } else {
                let _ = f.entry(&part.name, &value);
            }
        }
        f.finish()
    }
}

/// Iterator for [`Headers`], see [`Headers::iter`].
#[derive(Debug)]
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

/// Iterator for [`Headers`]'s header names, see [`Headers::names`].
#[derive(Debug)]
pub struct Names<'a> {
    headers: &'a Headers,
    pos: usize,
}

impl<'a> Iterator for Names<'a> {
    type Item = HeaderName<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.headers.parts.get(self.pos).map(|part| {
            self.pos += 1;
            part.name.borrow()
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

impl<'a> ExactSizeIterator for Names<'a> {
    fn len(&self) -> usize {
        self.headers.len() - self.pos
    }
}

impl<'a> FusedIterator for Names<'a> {}

/// HTTP header.
///
/// RFC 9110 section 6.3.
#[derive(Clone, PartialEq, Eq)]
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
        let _ = f.field("name", &self.name);
        if let Ok(str) = str::from_utf8(self.value) {
            let _ = f.field("value", &str);
        } else {
            let _ = f.field("value", &self.value);
        }
        f.finish()
    }
}

/// HTTP header name.
#[derive(Clone, PartialEq, Eq)]
pub struct HeaderName<'a> {
    /// The value MUST be lower case.
    inner: Str<'a>,
}

/// Macro to create [`Name`] constants.
macro_rules! known_headers {
    ($(
        $length: tt: [
            $( $(#[$meta: meta])* ( $const_name: ident, $http_name: expr ) $(,)? ),+
        ],
    )+) => {
        $($(
            $( #[$meta] )*
            pub const $const_name: HeaderName<'static> = HeaderName::from_lowercase($http_name);
        )+)+

        /// Create a new HTTP `HeaderName`.
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

impl<'n> HeaderName<'n> {
    // NOTE: these are automatically generated by the `src/parse_headers.bash`
    // script.
    // NOTE: we adding here also add to the
    // `functional::header::from_str_known_headers` test.
    known_headers!(
        2: [
            #[doc = "IM.\n\nRFC 3229."]
            (IM, "im"),
            #[doc = "If.\n\nRFC 4918."]
            (IF, "if"),
            #[doc = "TE.\n\nRFC 9110 section 10.1.4."]
            (TE, "te"),
        ],
        3: [
            #[doc = "Age.\n\nRFC 9111 section 5.1."]
            (AGE, "age"),
            #[doc = "DAV.\n\nRFC 4918."]
            (DAV, "dav"),
            #[doc = "TCN.\n\nRFC 2295."]
            (TCN, "tcn"),
            #[doc = "TTL.\n\nRFC 8030 section 5.2."]
            (TTL, "ttl"),
            #[doc = "Via.\n\nRFC 9110 section 7.6.3."]
            (VIA, "via"),
        ],
        4: [
            #[doc = "A-IM.\n\nRFC 3229."]
            (A_IM, "a-im"),
            #[doc = "ALPN.\n\nRFC 7639 section 2."]
            (ALPN, "alpn"),
            #[doc = "DASL.\n\nRFC 5323."]
            (DASL, "dasl"),
            #[doc = "DPoP.\n\nRFC -ietf-oauth-dpop-16."]
            (DPOP, "dpop"),
            #[doc = "Date.\n\nRFC 9110 section 6.6.1."]
            (DATE, "date"),
            #[doc = "ETag.\n\nRFC 9110 section 8.8.3."]
            (ETAG, "etag"),
            #[doc = "From.\n\nRFC 9110 section 10.1.2."]
            (FROM, "from"),
            #[doc = "Host.\n\nRFC 9110 section 7.2."]
            (HOST, "host"),
            #[doc = "Link.\n\nRFC 8288."]
            (LINK, "link"),
            #[doc = "SLUG.\n\nRFC 5023."]
            (SLUG, "slug"),
            #[doc = "Vary.\n\nRFC 9110 section 12.5.5."]
            (VARY, "vary"),
        ],
        5: [
            #[doc = "Allow.\n\nRFC 9110 section 10.2.1."]
            (ALLOW, "allow"),
            #[doc = "Close.\n\nRFC 9112 section 9.6."]
            (CLOSE, "close"),
            #[doc = "Depth.\n\nRFC 4918."]
            (DEPTH, "depth"),
            #[doc = "Label.\n\nRFC 3253."]
            (LABEL, "label"),
            #[doc = "Meter.\n\nRFC 2227."]
            (METER, "meter"),
            #[doc = "Range.\n\nRFC 9110 section 14.2."]
            (RANGE, "range"),
            #[doc = "Topic.\n\nRFC 8030 section 5.4."]
            (TOPIC, "topic"),
        ],
        6: [
            #[doc = "Accept.\n\nRFC 9110 section 12.5.1."]
            (ACCEPT, "accept"),
            #[doc = "Cookie.\n\nRFC 6265."]
            (COOKIE, "cookie"),
            #[doc = "Digest.\n\nRFC 3230."]
            (DIGEST, "digest"),
            #[doc = "Expect.\n\nRFC 9110 section 10.1.1."]
            (EXPECT, "expect"),
            #[doc = "OSCORE.\n\nRFC 8613 section 11.1."]
            (OSCORE, "oscore"),
            #[doc = "Origin.\n\nRFC 6454."]
            (ORIGIN, "origin"),
            #[doc = "Prefer.\n\nRFC 7240."]
            (PREFER, "prefer"),
            #[doc = "Server.\n\nRFC 9110 section 10.2.4."]
            (SERVER, "server"),
            #[doc = "Sunset.\n\nRFC 8594."]
            (SUNSET, "sunset"),
        ],
        7: [
            #[doc = "Alt-Svc.\n\nRFC 7838."]
            (ALT_SVC, "alt-svc"),
            #[doc = "Expires.\n\nRFC 9111 section 5.3."]
            (EXPIRES, "expires"),
            #[doc = "Hobareg.\n\nRFC 7486 section 6.1.1."]
            (HOBAREG, "hobareg"),
            #[doc = "Ping-To.\n\nHTML."]
            (PING_TO, "ping-to"),
            #[doc = "Referer.\n\nRFC 9110 section 10.1.3."]
            (REFERER, "referer"),
            #[doc = "Refresh.\n\nHTML."]
            (REFRESH, "refresh"),
            #[doc = "Timeout.\n\nRFC 4918."]
            (TIMEOUT, "timeout"),
            #[doc = "Trailer.\n\nRFC 9110 section 6.6.2."]
            (TRAILER, "trailer"),
            #[doc = "Upgrade.\n\nRFC 9110 section 7.8."]
            (UPGRADE, "upgrade"),
            #[doc = "Urgency.\n\nRFC 8030 section 5.3."]
            (URGENCY, "urgency"),
        ],
        8: [
            #[doc = "Alt-Used.\n\nRFC 7838."]
            (ALT_USED, "alt-used"),
            #[doc = "CDN-Loop.\n\nRFC 8586."]
            (CDN_LOOP, "cdn-loop"),
            #[doc = "If-Match.\n\nRFC 9110 section 13.1.1."]
            (IF_MATCH, "if-match"),
            #[doc = "If-Range.\n\nRFC 9110 section 13.1.5."]
            (IF_RANGE, "if-range"),
            #[doc = "Location.\n\nRFC 9110 section 10.2.2."]
            (LOCATION, "location"),
            #[doc = "Position.\n\nRFC 3648."]
            (POSITION, "position"),
            #[doc = "Priority.\n\nRFC 9218."]
            (PRIORITY, "priority"),
        ],
        9: [
            #[doc = "Accept-CH.\n\nRFC 8942 section 3.1."]
            (ACCEPT_CH, "accept-ch"),
            #[doc = "Expect-CT.\n\nRFC 9163."]
            (EXPECT_CT, "expect-ct"),
            #[doc = "Forwarded.\n\nRFC 7239."]
            (FORWARDED, "forwarded"),
            #[doc = "Negotiate.\n\nRFC 2295."]
            (NEGOTIATE, "negotiate"),
            #[doc = "Overwrite.\n\nRFC 4918."]
            (OVERWRITE, "overwrite"),
            #[doc = "Ping-From.\n\nHTML."]
            (PING_FROM, "ping-from"),
        ],
        10: [
            #[doc = "Alternates.\n\nRFC 2295."]
            (ALTERNATES, "alternates"),
            #[doc = "Connection.\n\nRFC 9110 section 7.6.1."]
            (CONNECTION, "connection"),
            #[doc = "Content-ID.\n\nThe HTTP Distribution and Replication Protocol."]
            (CONTENT_ID, "content-id"),
            #[doc = "DPoP-Nonce.\n\nRFC -ietf-oauth-dpop-16."]
            (DPOP_NONCE, "dpop-nonce"),
            #[doc = "Delta-Base.\n\nRFC 3229."]
            (DELTA_BASE, "delta-base"),
            #[doc = "Early-Data.\n\nRFC 8470."]
            (EARLY_DATA, "early-data"),
            #[doc = "Keep-Alive.\n\nRFC 2068."]
            (KEEP_ALIVE, "keep-alive"),
            #[doc = "Lock-Token.\n\nRFC 4918."]
            (LOCK_TOKEN, "lock-token"),
            #[doc = "Set-Cookie.\n\nRFC 6265."]
            (SET_COOKIE, "set-cookie"),
            #[doc = "SoapAction.\n\nSimple Object Access Protocol (SOAP) 1.1."]
            (SOAPACTION, "soapaction"),
            #[doc = "Status-URI.\n\nRFC 2518."]
            (STATUS_URI, "status-uri"),
            #[doc = "Tracestate.\n\nTrace Context."]
            (TRACESTATE, "tracestate"),
            #[doc = "User-Agent.\n\nRFC 9110 section 10.1.5."]
            (USER_AGENT, "user-agent"),
        ],
        11: [
            #[doc = "Accept-Post.\n\nLinked Data Platform 1.0."]
            (ACCEPT_POST, "accept-post"),
            #[doc = "Client-Cert.\n\nRFC -ietf-httpbis-client-cert-field-06 section 2."]
            (CLIENT_CERT, "client-cert"),
            #[doc = "Destination.\n\nRFC 4918."]
            (DESTINATION, "destination"),
            #[doc = "Retry-After.\n\nRFC 9110 section 10.2.3."]
            (RETRY_AFTER, "retry-after"),
            #[doc = "Sec-Purpose.\n\nFetch."]
            (SEC_PURPOSE, "sec-purpose"),
            #[doc = "Traceparent.\n\nTrace Context."]
            (TRACEPARENT, "traceparent"),
            #[doc = "Want-Digest.\n\nRFC 3230."]
            (WANT_DIGEST, "want-digest"),
        ],
        12: [
            #[doc = "Accept-Patch.\n\nRFC 5789."]
            (ACCEPT_PATCH, "accept-patch"),
            #[doc = "Cache-Status.\n\nRFC 9211."]
            (CACHE_STATUS, "cache-status"),
            #[doc = "Content-Type.\n\nRFC 9110 section 8.3."]
            (CONTENT_TYPE, "content-type"),
            #[doc = "MIME-Version.\n\nRFC 9112 Appendix B.1."]
            (MIME_VERSION, "mime-version"),
            #[doc = "Max-Forwards.\n\nRFC 9110 section 7.6.2."]
            (MAX_FORWARDS, "max-forwards"),
            #[doc = "Proxy-Status.\n\nRFC 9209."]
            (PROXY_STATUS, "proxy-status"),
            #[doc = "Redirect-Ref.\n\nRFC 4437."]
            (REDIRECT_REF, "redirect-ref"),
            #[doc = "Replay-Nonce.\n\nRFC 8555 section 6.5.1."]
            (REPLAY_NONCE, "replay-nonce"),
            #[doc = "Schedule-Tag.\n\nRFC 6338."]
            (SCHEDULE_TAG, "schedule-tag"),
            #[doc = "Variant-Vary.\n\nRFC 2295."]
            (VARIANT_VARY, "variant-vary"),
            #[doc = "X-Request-ID."]
            (X_REQUEST_ID, "x-request-id"),
        ],
        13: [
            #[doc = "Accept-Ranges.\n\nRFC 9110 section 14.3."]
            (ACCEPT_RANGES, "accept-ranges"),
            #[doc = "Authorization.\n\nRFC 9110 section 11.6.2."]
            (AUTHORIZATION, "authorization"),
            #[doc = "Cache-Control.\n\nRFC 9111 section 5.2."]
            (CACHE_CONTROL, "cache-control"),
            #[doc = "Content-Range.\n\nRFC 9110 section 14.4."]
            (CONTENT_RANGE, "content-range"),
            #[doc = "If-None-Match.\n\nRFC 9110 section 13.1.2."]
            (IF_NONE_MATCH, "if-none-match"),
            #[doc = "Last-Event-ID.\n\nHTML."]
            (LAST_EVENT_ID, "last-event-id"),
            #[doc = "Last-Modified.\n\nRFC 9110 section 8.8.2."]
            (LAST_MODIFIED, "last-modified"),
            #[doc = "OData-Version.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ODATA_VERSION, "odata-version"),
            #[doc = "Ordering-Type.\n\nRFC 3648."]
            (ORDERING_TYPE, "ordering-type"),
            #[doc = "Server-Timing.\n\nServer Timing."]
            (SERVER_TIMING, "server-timing"),
        ],
        14: [
            #[doc = "Cal-Managed-ID.\n\nRFC 8607 section 5.1."]
            (CAL_MANAGED_ID, "cal-managed-id"),
            #[doc = "Cert-Not-After.\n\nRFC 8739 section 3.3."]
            (CERT_NOT_AFTER, "cert-not-after"),
            #[doc = "Content-Length.\n\nRFC 9110 section 8.6."]
            (CONTENT_LENGTH, "content-length"),
            #[doc = "OData-EntityId.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ODATA_ENTITYID, "odata-entityid"),
            #[doc = "Schedule-Reply.\n\nRFC 6638."]
            (SCHEDULE_REPLY, "schedule-reply"),
        ],
        15: [
            #[doc = "Accept-Datetime.\n\nRFC 7089."]
            (ACCEPT_DATETIME, "accept-datetime"),
            #[doc = "Accept-Encoding.\n\nRFC 9110 section 12.5.3."]
            (ACCEPT_ENCODING, "accept-encoding"),
            #[doc = "Accept-Features.\n\nRFC 2295."]
            (ACCEPT_FEATURES, "accept-features"),
            #[doc = "Accept-Language.\n\nRFC 9110 section 12.5.4."]
            (ACCEPT_LANGUAGE, "accept-language"),
            #[doc = "Cert-Not-Before.\n\nRFC 8739 section 3.3."]
            (CERT_NOT_BEFORE, "cert-not-before"),
            #[doc = "Clear-Site-Data.\n\nClear Site Data."]
            (CLEAR_SITE_DATA, "clear-site-data"),
            #[doc = "Differential-ID.\n\nThe HTTP Distribution and Replication Protocol."]
            (DIFFERENTIAL_ID, "differential-id"),
            #[doc = "OData-Isolation.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ODATA_ISOLATION, "odata-isolation"),
            #[doc = "Public-Key-Pins.\n\nRFC 7469."]
            (PUBLIC_KEY_PINS, "public-key-pins"),
            #[doc = "X-Frame-Options.\n\nHTML."]
            (X_FRAME_OPTIONS, "x-frame-options"),
        ],
        16: [
            #[doc = "Accept-Additions.\n\nRFC 2324."]
            (ACCEPT_ADDITIONS, "accept-additions"),
            #[doc = "CalDAV-Timezones.\n\nRFC 7809 section 7.1."]
            (CALDAV_TIMEZONES, "caldav-timezones"),
            #[doc = "Capsule-Protocol.\n\nRFC 9297."]
            (CAPSULE_PROTOCOL, "capsule-protocol"),
            #[doc = "Content-Encoding.\n\nRFC 9110 section 8.4."]
            (CONTENT_ENCODING, "content-encoding"),
            #[doc = "Content-Language.\n\nRFC 9110 section 8.5."]
            (CONTENT_LANGUAGE, "content-language"),
            #[doc = "Content-Location.\n\nRFC 9110 section 8.7."]
            (CONTENT_LOCATION, "content-location"),
            #[doc = "Memento-Datetime.\n\nRFC 7089."]
            (MEMENTO_DATETIME, "memento-datetime"),
            #[doc = "OData-MaxVersion.\n\nOData Version 4.01 Part 1: Protocol, OASIS, Chet_Ensign."]
            (ODATA_MAXVERSION, "odata-maxversion"),
            #[doc = "WWW-Authenticate.\n\nRFC 9110 section 11.6.1."]
            (WWW_AUTHENTICATE, "www-authenticate"),
        ],
        17: [
            #[doc = "CDN-Cache-Control.\n\nRFC 9213."]
            (CDN_CACHE_CONTROL, "cdn-cache-control"),
            #[doc = "Client-Cert-Chain.\n\nRFC -ietf-httpbis-client-cert-field-06 section 2."]
            (CLIENT_CERT_CHAIN, "client-cert-chain"),
            #[doc = "If-Modified-Since.\n\nRFC 9110 section 13.1.3."]
            (IF_MODIFIED_SINCE, "if-modified-since"),
            #[doc = "OSLC-Core-Version.\n\nOASIS Project Specification 01, OASIS, Chet_Ensign."]
            (OSLC_CORE_VERSION, "oslc-core-version"),
            #[doc = "Sec-Token-Binding.\n\nRFC 8473."]
            (SEC_TOKEN_BINDING, "sec-token-binding"),
            #[doc = "Sec-WebSocket-Key.\n\nRFC 6455."]
            (SEC_WEBSOCKET_KEY, "sec-websocket-key"),
            #[doc = "Surrogate-Control.\n\nEdge Architecture Specification."]
            (SURROGATE_CONTROL, "surrogate-control"),
            #[doc = "Transfer-Encoding.\n\nRFC 9112 section 6.1."]
            (TRANSFER_ENCODING, "transfer-encoding"),
        ],
        18: [
            #[doc = "Preference-Applied.\n\nRFC 7240."]
            (PREFERENCE_APPLIED, "preference-applied"),
            #[doc = "Proxy-Authenticate.\n\nRFC 9110 section 11.7.1."]
            (PROXY_AUTHENTICATE, "proxy-authenticate"),
        ],
        19: [
            #[doc = "Authentication-Info.\n\nRFC 9110 section 11.6.3."]
            (AUTHENTICATION_INFO, "authentication-info"),
            #[doc = "Content-Disposition.\n\nRFC 6266."]
            (CONTENT_DISPOSITION, "content-disposition"),
            #[doc = "If-Unmodified-Since.\n\nRFC 9110 section 13.1.4."]
            (IF_UNMODIFIED_SINCE, "if-unmodified-since"),
            #[doc = "Proxy-Authorization.\n\nRFC 9110 section 11.7.2."]
            (PROXY_AUTHORIZATION, "proxy-authorization"),
        ],
        20: [
            #[doc = "Origin-Agent-Cluster.\n\nHTML."]
            (ORIGIN_AGENT_CLUSTER, "origin-agent-cluster"),
            #[doc = "Sec-WebSocket-Accept.\n\nRFC 6455."]
            (SEC_WEBSOCKET_ACCEPT, "sec-websocket-accept"),
            #[doc = "Surrogate-Capability.\n\nEdge Architecture Specification."]
            (SURROGATE_CAPABILITY, "surrogate-capability"),
        ],
        21: [
            #[doc = "Apply-To-Redirect-Ref.\n\nRFC 4437."]
            (APPLY_TO_REDIRECT_REF, "apply-to-redirect-ref"),
            #[doc = "If-Schedule-Tag-Match.\n\n RFC 6338: Scheduling Extensions to CalDAV."]
            (IF_SCHEDULE_TAG_MATCH, "if-schedule-tag-match"),
            #[doc = "Sec-WebSocket-Version.\n\nRFC 6455."]
            (SEC_WEBSOCKET_VERSION, "sec-websocket-version"),
        ],
        22: [
            #[doc = "Access-Control-Max-Age.\n\nFetch."]
            (ACCESS_CONTROL_MAX_AGE, "access-control-max-age"),
            #[doc = "Authentication-Control.\n\nRFC 8053 section 4."]
            (AUTHENTICATION_CONTROL, "authentication-control"),
            #[doc = "Sec-WebSocket-Protocol.\n\nRFC 6455."]
            (SEC_WEBSOCKET_PROTOCOL, "sec-websocket-protocol"),
            #[doc = "X-Content-Type-Options.\n\nFetch."]
            (X_CONTENT_TYPE_OPTIONS, "x-content-type-options"),
        ],
        23: [
            #[doc = "Content-Security-Policy.\n\nContent Security Policy Level 3."]
            (CONTENT_SECURITY_POLICY, "content-security-policy"),
        ],
        24: [
            #[doc = "Sec-WebSocket-Extensions.\n\nRFC 6455."]
            (SEC_WEBSOCKET_EXTENSIONS, "sec-websocket-extensions"),
        ],
        25: [
            #[doc = "Optional-WWW-Authenticate.\n\nRFC 8053 section 3."]
            (OPTIONAL_WWW_AUTHENTICATE, "optional-www-authenticate"),
            #[doc = "Proxy-Authentication-Info.\n\nRFC 9110 section 11.7.3."]
            (PROXY_AUTHENTICATION_INFO, "proxy-authentication-info"),
            #[doc = "Strict-Transport-Security.\n\nRFC 6797."]
            (STRICT_TRANSPORT_SECURITY, "strict-transport-security"),
        ],
        26: [
            #[doc = "Cross-Origin-Opener-Policy.\n\nHTML."]
            (CROSS_ORIGIN_OPENER_POLICY, "cross-origin-opener-policy"),
        ],
        27: [
            #[doc = "Access-Control-Allow-Origin.\n\nFetch."]
            (ACCESS_CONTROL_ALLOW_ORIGIN, "access-control-allow-origin"),
            #[doc = "Public-Key-Pins-Report-Only.\n\nRFC 7469."]
            (PUBLIC_KEY_PINS_REPORT_ONLY, "public-key-pins-report-only"),
        ],
        28: [
            #[doc = "Access-Control-Allow-Headers.\n\nFetch."]
            (ACCESS_CONTROL_ALLOW_HEADERS, "access-control-allow-headers"),
            #[doc = "Access-Control-Allow-Methods.\n\nFetch."]
            (ACCESS_CONTROL_ALLOW_METHODS, "access-control-allow-methods"),
            #[doc = "Cross-Origin-Embedder-Policy.\n\nHTML."]
            (CROSS_ORIGIN_EMBEDDER_POLICY, "cross-origin-embedder-policy"),
            #[doc = "Cross-Origin-Resource-Policy.\n\nFetch."]
            (CROSS_ORIGIN_RESOURCE_POLICY, "cross-origin-resource-policy"),
        ],
        29: [
            #[doc = "Access-Control-Expose-Headers.\n\nFetch."]
            (ACCESS_CONTROL_EXPOSE_HEADERS, "access-control-expose-headers"),
            #[doc = "Access-Control-Request-Method.\n\nFetch."]
            (ACCESS_CONTROL_REQUEST_METHOD, "access-control-request-method"),
        ],
        30: [
            #[doc = "Access-Control-Request-Headers.\n\nFetch."]
            (ACCESS_CONTROL_REQUEST_HEADERS, "access-control-request-headers"),
        ],
        32: [
            #[doc = "Access-Control-Allow-Credentials.\n\nFetch."]
            (ACCESS_CONTROL_ALLOW_CREDENTIALS, "access-control-allow-credentials"),
        ],
        33: [
            #[doc = "Include-Referred-Token-Binding-ID.\n\nRFC 8473."]
            (INCLUDE_REFERRED_TOKEN_BINDING_ID, "include-referred-token-binding-id"),
        ],
        35: [
            #[doc = "Content-Security-Policy-Report-Only.\n\nContent Security Policy Level 3."]
            (CONTENT_SECURITY_POLICY_REPORT_ONLY, "content-security-policy-report-only"),
        ],
        38: [
            #[doc = "Cross-Origin-Opener-Policy-Report-Only.\n\nHTML."]
            (CROSS_ORIGIN_OPENER_POLICY_REPORT_ONLY, "cross-origin-opener-policy-report-only"),
        ],
        40: [
            #[doc = "Cross-Origin-Embedder-Policy-Report-Only.\n\nHTML."]
            (CROSS_ORIGIN_EMBEDDER_POLICY_REPORT_ONLY, "cross-origin-embedder-policy-report-only"),
        ],
    );

    /// Create a new HTTP `HeaderName`.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not all ASCII lowercase.
    pub const fn from_lowercase(name: &'n str) -> HeaderName<'n> {
        assert!(is_lower_case(name), "header name not lowercase");
        HeaderName {
            inner: Str::from_str(name),
        }
    }

    /// Borrow the header name for a shorter lifetime.
    ///
    /// This is used in things like [`Headers::get`] and [`Iter`] for `Headers`
    /// to avoid clone heap-allocated `HeaderName`s.
    fn borrow<'b>(&'b self) -> HeaderName<'b> {
        HeaderName {
            inner: self.inner.borrow(),
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
        self.inner.is_heap_allocated()
    }
}

impl From<String> for HeaderName<'static> {
    fn from(mut name: String) -> HeaderName<'static> {
        name.make_ascii_lowercase();
        HeaderName {
            inner: Str::from_string(name),
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
/// The main use case for this trait is [`Header::parse`]. Because of this the
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

/// Error returned by the [`FromHeaderValue`] implementation for numbers, e.g.
/// `usize`.
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
                    if b.is_ascii_digit() {
                        match value.checked_mul(10) {
                            Some(v) => value = v,
                            None => return Err(ParseIntError),
                        }
                        match value.checked_add(<$ty>::from(b - b'0')) {
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

/// Error returned by the [`FromHeaderValue`] implementation for [`SystemTime`].
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
