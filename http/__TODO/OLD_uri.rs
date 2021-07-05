
        /*
        RFC 7230 5.3:
          authority-form = authority
          absolute-form  = absolute-URI

         4.3.  Absolute URI RFC 3986.

           absolute-URI  = scheme ":" hier-part [ "?" query ]

           scheme      = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )

           hier-part   = "//" authority path-abempty
                       / path-absolute
                       / path-rootless
                       / path-empty


          path-absolute = "/" [ segment-nz *( "/" segment ) ]
          path-rootless = segment-nz *( "/" segment )
          path-empty    = 0<pchar>


           authority   = host [ ":" port ]

           path-abempty  = *( "/" segment )

              segment       = *pchar

              pchar         = unreserved / pct-encoded / sub-delims / ":" / "@"

         */


        // TODO: how to determine in what form the uri is:
        // authority-form = authority OR
        // absolute-form  = absolute-URI ?

        // Now we only have two forms left `absolute-form` and `authority-form`
        // per RFC 7230 section 5.3.
        let mut path_start = None;
        let mut query_start = None;
        let mut char_indices = uri.char_indices().peekable();

        while let Some((idx, c)) = char_indices.next() {
            match c {
                // RFC 3986 section 3.1:
                // > scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
                '0'..='9' | 'a'..='z' | 'A'..='Z' | '+' | '-' | '.' => {
                    todo!("valid scheme");
                }
                _ => todo!(),
            }
        }

        /*
        while let Some((idx, c)) = char_indices.next() {
            match c {
                // Anything before the first `:` is the scheme.
                ':' => {
                    if scheme_end.is_none() {
                        scheme_end = NonZeroU16::new(idx as u16);

                        if let Some((_, '/')) = char_indices.peek() {
                            char_indices.next();
                            if let Some((_, '/')) = char_indices.peek() {
                                // TODO: parse authority.
                                todo!()
                            } else {
                                todo!();
                            }
                        }
                    } else {
                        todo!("invalid ':'")
                    }
                }
                // TODO.
                '/' if path_start.is_none() => path_start = Some(idx as u16),
                '?' if query_start.is_none() => query_start = NonZeroU16::new(idx as u16),
                '?' => todo!("not allowed (?)"),
                _ => {}
            }
        }
        */

        // FIXME: no unwrap.
        let path_start = path_start.unwrap();
        // FIXME: change to errors.
        debug_assert!(scheme_end.map_or(true, |n| n.get() < path_start));
        debug_assert!(query_start.map_or(true, |n: NonZeroU16| n.get() > path_start));
        // FIXME: host_end > host_start.
        // FIXME: host_end > path_start.

        Uri {
            complete: uri.into(),
            scheme_end,
            host_start,
            host_end,
            port_end,
            path_start,
            query_start,
        }
