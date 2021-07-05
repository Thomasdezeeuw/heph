#![allow(
    unused_variables,
    unused_mut,
    unreachable_code,
    dead_code,
    unreachable_patterns
)] // FIXME: remove.

use std::iter::Peekable;
use std::num::NonZeroU16;
use std::str::CharIndices;

/// Uniform Resource Identifier (URI).
///
/// URIs are build up from a number of following components
///
/// An example from RFC 3986 section 5:
///
/// ```text
///            host     port
///           ___|____    __
///          /         \ /  \
///   http://example.com:8042/some/path?query=value&abc=123
///   \__/   \______________/\________/ \_________________/
///    |           |             |               |
/// scheme     authority        path           query
/// ```
///
/// RFC 7230 section 5.3, RFC 3986.
///
/// # Notes
///
/// The size of the URI is limited to 65k (`1 << 16`) bytes.
///
/// This does not include all parts of an URI as specified by RFC 3986, it only
/// include the parts as required by RFC 7230.
#[derive(Debug)]
pub struct Uri {
    complete: Box<str>,
    // Scheme if `Some`, starts at index 0.
    scheme_end: Option<NonZeroU16>,
    host_start: Option<NonZeroU16>,
    host_and_authority_end: Option<(NonZeroU16, NonZeroU16)>,
    port: Option<NonZeroU16>,
    // Start of the path.
    path_start: u16,
    query_start: Option<NonZeroU16>,
}

impl Uri {
    const MAX_LEN: usize = u16::MAX as usize;

    pub fn new(uri: String) -> Uri {
        // This referens to names using in RFC 7230 and RFC 3986, it's handy to
        // have a copy of those close by when reading this code.

        match uri.len() {
            // FIXME: return an error.
            0 => todo!("empty Uri"),
            // FIXME: return an error.
            n if n >= u16::MAX as usize => todo!("Uri too long"),
            _ => {}
        }

        let mut scheme_end = None;
        let mut host_start = None;
        let mut host_and_authority_end = None;
        let mut port = None;
        let mut path_start = 0;
        let mut query_start = None;

        // Check for `origin-form` and `asterisk-form`.
        // SAFETY: `uri` is not empty, checked above.
        match uri.as_bytes()[0] {
            // URI is in `origin-form`, containing only a path and query.
            b'/' => {
                // SAFETY: checked length above.
                query_start = find_query_start(&uri, 1);
            }
            // URI is in `asterisk-form`, which is just an asterisk (`*`).
            b'*' if uri.len() == 1 => {}
            b'a'..=b'z' | b'A'..=b'Z' => {
                // URI might be in absolute-form. From RFC 7230 section 5.3.2:
                // > absolute-form = absolute-URI
                // From RFC 3986 section 4.3:
                // > absolute-URI = scheme ":" hier-part [ "?" query ]
                // From RFC 3986 section 3.1:
                // > scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )

                let mut char_indices = uri.char_indices().peekable();
                char_indices.next(); // Ignore the first byte (already checked it).

                // The `scheme ":"` part of `absolute-URI`.
                match find_scheme_end(&mut char_indices) {
                    Ok(Some(idx)) => scheme_end = Some(idx),
                    Ok(None) => todo!("no scheme end"),
                    Err(c) => todo!("invalid byte: {}", c),
                }

                // The `hier-part` of `absolute-URI`.
                // From RFC 3986 section 3:
                // > hier-part = "//" authority path-abempty
                // >             / path-absolute
                // >             / path-rootless
                // >             / path-empty
                path_start = uri.len() as u16;
                match char_indices.next() {
                    Some((idx, '/')) if let Some((idx, '/')) = char_indices.peek() => {
                        // The `"//" authority path-abempty` part of
                        // `hier-part`.
                        char_indices.next(); // Remove second `/`.
                        parse_authority(&mut char_indices, &mut host_start, &mut host_and_authority_end, &mut port);

                        // The `path-abempty` part.
                        if let Some((idx, _)) = char_indices.next() {
                            path_start = idx as u16;
                        }
                    }
                    Some((idx, '/'))  => {
                        // path-absolute = "/" [ segment-nz *( "/" segment ) ]
                        // segment-nz    = 1*pchar
                        todo!("maybe path-absolute");
                    }
                    _ => {
                        // path-rootless = segment-nz *( "/" segment )
                        // segment-nz    = 1*pchar
                    },
                    None => {
                        // The `path-empty` part of `hier-part`.
                    }
                    _ => todo!(),
                }

                // The `[ "?" query ]` of `absolute-URI`.
                query_start = find_query_start(&uri, 1);
            }
            /*
            // FIXME: this has overlap with `absolute-form` above.
            // Start of host =  IP-literal / IPv4address / reg-name
            b'[' /* Start of "IP-literal". */ | b'0' ..= b'9' /* Start of "IPv4address". */ |
                // Start of reg-name = *( unreserved / pct-encoded / sub-delims )
                b'a'..=b'z' | b'A'..=b'Z' | /* NOTE: have DIGIT above. */ b'-' | b'.' | b'_' | b'~' | /* unreserved */
                b'%' | /* pct-encoded. */
                b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' /* sub-delims. */
            => {
                // URI might be in authority-form, which starts with a `host`.
                // See RFC 3986 3.2.2 for definition of `host`.
                todo!("maybe absolute-form")
            },
            */
            _ => todo!("invalid byte"),
        }

        Uri {
            complete: uri.into(),
            scheme_end,
            host_start,
            host_and_authority_end,
            port,
            path_start,
            query_start,
        }
    }

    /// Returns the scheme of the URI, if any.
    pub fn scheme(&self) -> Option<&str> {
        match self.scheme_end {
            Some(end) => Some(&self.complete[0..end.get() as usize]),
            None => None,
        }
    }

    /// Returns the authority of the URI, if any.
    pub fn authority(&self) -> Option<&str> {
        match (self.host_start, self.host_and_authority_end) {
            (Some(start), Some((_, end))) => {
                Some(&self.complete[start.get() as usize..=end.get() as usize])
            }
            _ => None,
        }
    }

    pub fn host(&self) -> Option<&str> {
        match (self.host_start, self.host_and_authority_end) {
            (Some(start), Some((end, _))) => {
                let mut start = start.get() as usize;
                if self.complete.as_bytes()[start] == b'[' {
                    start += 1;
                }
                Some(&self.complete[start..end.get() as usize])
            }
            _ => None,
        }
    }

    /// Returns the port, if provided.
    pub fn port(&self) -> Option<u16> {
        self.port.map(|n| n.get())
    }

    /// Returns the path of the URI.
    pub fn path(&self) -> &str {
        match self.query_start {
            Some(end) => &self.complete[self.path_start as usize..end.get() as usize],
            None => &self.complete[self.path_start as usize..],
        }
    }

    /// Returns the query of the URI, if any.
    pub fn query(&self) -> Option<&str> {
        match self.query_start {
            Some(start) => Some(&self.complete[start.get() as usize + 1..]),
            None => None,
        }
    }

    /// Returns `true` if the URI is in origin form.
    pub fn is_origin(&self) -> bool {
        // TODO.
        todo!()
    }

    /// Returns `true` if the URI is in absolute form.
    pub fn is_absolute(&self) -> bool {
        // TODO.
        todo!()
    }

    /// Returns `true` if the URI is in authority form.
    pub fn is_authority(&self) -> bool {
        // TODO.
        todo!()
    }

    /// Returns `true` if the URI is in asterisk form.
    ///
    /// RFC 7230 section 5.3.4.
    pub fn is_asterisk(&self) -> bool {
        *self.complete == *"*"
    }
}

// SAFETY: the function below except the URI to be no larger then
// `Uri::MAX_LEN`.

/// Find the start of the query (i.e. `?`).
///
/// `uri` must be be smaller then `Uri::MAX_LEN`.
fn find_query_start(uri: &str, offset: usize) -> Option<NonZeroU16> {
    debug_assert!(uri.len() < Uri::MAX_LEN);
    // SAFETY: checked length above.
    uri[offset..]
        .find('?')
        .and_then(|idx| NonZeroU16::new((idx + offset) as u16))
}

type CharIter<'a> = Peekable<CharIndices<'a>>;

/// Returns the index where the `scheme` ends, `None` if it doesn't end, or an
/// error if an invalid character is found.
fn find_scheme_end(char_indices: &mut CharIter<'_>) -> Result<Option<NonZeroU16>, char> {
    // From RFC 3986 section 3.1:
    // > scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
    while let Some((idx, c)) = char_indices.next() {
        match c {
            // Valid scheme character, see RFC 3986 section 3.1 or
            // above.
            '0'..='9' | 'a'..='z' | 'A'..='Z' | '+' | '-' | '.' => {}
            ':' => return Ok(NonZeroU16::new(idx as u16)),
            c => return Err(c),
        }
    }
    Ok(None)
}

fn parse_authority(
    char_indices: &mut CharIter<'_>,
    host_start: &mut Option<NonZeroU16>,
    host_and_authority_end: &mut Option<(NonZeroU16, NonZeroU16)>,
    port: &mut Option<NonZeroU16>,
) {
    // From RFC 3986 section 3.2:
    // > authority = [ userinfo "@" ] host [ ":" port ]

    // FIXME: parse `userinfo`

    // From RFC 3986 section 3.2.2:
    // > host = IP-literal / IPv4address / reg-name

    // TODO: remove
    // > IP-literal = "[" ( IPv6address / IPvFuture  ) "]"
    // > IPv6address =                            6( h16 ":" ) ls32
    // >             /                       "::" 5( h16 ":" ) ls32
    // >             / [               h16 ] "::" 4( h16 ":" ) ls32
    // >             / [ *1( h16 ":" ) h16 ] "::" 3( h16 ":" ) ls32
    // >             / [ *2( h16 ":" ) h16 ] "::" 2( h16 ":" ) ls32
    // >             / [ *3( h16 ":" ) h16 ] "::"    h16 ":"   ls32
    // >             / [ *4( h16 ":" ) h16 ] "::"              ls32
    // >             / [ *5( h16 ":" ) h16 ] "::"              h16
    // >             / [ *6( h16 ":" ) h16 ] "::"
    // >
    // > ls32        = ( h16 ":" h16 ) / IPv4address
    // >             ; least-significant 32 bits of address
    // > h16         = 1*4HEXDIG
    // >             ; 16 bits of address represented in hexadecimal
    // >
    // > IPvFuture  = "v" 1*HEXDIG "." 1*( unreserved / sub-delims / ":" )
    // TODO: END remove

    // > IPv4address = dec-octet "." dec-octet "." dec-octet "." dec-octet
    //
    // From RFC 3986 section 1.3:
    // > HEXDIG (hexadecimal digits)
    //
    // TODO: reg-name
    match char_indices.next() {
        Some((start, b)) => match b {
            '[' => {
                *host_start = NonZeroU16::new(start as u16);
                // Start of `IP-literal`.
                // From RFC 3986 section 3.2.2:
                // > IP-literal = "[" ( IPv6address / IPvFuture  ) "]"
                while let Some((idx, c)) = char_indices.next() {
                    match c {
                        ']' => {
                            let end = NonZeroU16::new(idx as u16);
                            *host_and_authority_end = end.map(|n| (n, n));
                            break;
                        }
                        _ => {}
                    }
                }

                if host_and_authority_end.is_none() {
                    todo!("no end to IP-literal");
                }
            }

            /*
                | '0' ..= '9' /* Start of "IPv4address". */ |
                // Start of reg-name = *( unreserved / pct-encoded / sub-delims )
                'a'..='z' | 'A'..='Z' | /* NOTE: have DIGIT above. */ '-' | '.' | '_' | '~' | /* unreserved */
                '%' | /* pct-encoded. */
                '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '=' /* sub-delims. */
            => {
                todo!()
            },
            */
            _ => todo!(),
        },
        None => todo!(),
    }

    // The `[ ":" port ]` of `authority`.
    if let Some((mut authority_end, ':')) = char_indices.peek() {
        // From RFC 3986 section 3.2.3:
        // > port = *DIGIT
        char_indices.next(); // Remove `:`.
        let mut parsed_port: u16 = 0;
        loop {
            match char_indices.peek().copied() {
                Some((idx, c)) if matches!(c, '0'..='9') => {
                    // Valid port.
                    char_indices.next();
                    authority_end = idx;

                    match parsed_port.checked_mul(10) {
                        Some(p) => parsed_port = p,
                        None => todo!("port overflow"),
                    }
                    match parsed_port.checked_add((c as u8 - b'0') as u16) {
                        Some(p) => parsed_port = p,
                        None => todo!("port overflow"),
                    }
                }
                _ => break,
            }
        }
        host_and_authority_end.as_mut().unwrap().1 = NonZeroU16::new(authority_end as u16).unwrap();
        // If the `parsed_port` is zero this will set `port` to `None`, which is
        // what we want.
        *port = NonZeroU16::new(parsed_port);
    }
}
