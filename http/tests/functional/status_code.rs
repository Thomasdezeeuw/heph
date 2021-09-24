use heph_http::StatusCode;

#[test]
fn is_informational() {
    let informational = &[100, 101, 199];
    for status in informational {
        assert!(StatusCode(*status).is_informational());
    }
    let not_informational = &[0, 10, 200, 201, 400, 999];
    for status in not_informational {
        assert!(!StatusCode(*status).is_informational());
    }
}

#[test]
fn is_successful() {
    let successful = &[200, 201, 299];
    for status in successful {
        assert!(StatusCode(*status).is_successful());
    }
    let not_successful = &[0, 10, 100, 101, 400, 999];
    for status in not_successful {
        assert!(!StatusCode(*status).is_successful());
    }
}

#[test]
fn is_redirect() {
    let redirect = &[300, 301, 399];
    for status in redirect {
        assert!(StatusCode(*status).is_redirect());
    }
    let not_redirect = &[0, 10, 100, 101, 400, 999];
    for status in not_redirect {
        assert!(!StatusCode(*status).is_redirect());
    }
}

#[test]
fn is_client_error() {
    let client_error = &[400, 401, 499];
    for status in client_error {
        assert!(StatusCode(*status).is_client_error());
    }
    let not_client_error = &[0, 10, 100, 101, 300, 500, 999];
    for status in not_client_error {
        assert!(!StatusCode(*status).is_client_error());
    }
}

#[test]
fn is_server_error() {
    let server_error = &[500, 501, 599];
    for status in server_error {
        assert!(StatusCode(*status).is_server_error());
    }
    let not_server_error = &[0, 10, 100, 101, 400, 600, 999];
    for status in not_server_error {
        assert!(!StatusCode(*status).is_server_error());
    }
}

#[test]
fn includes_body() {
    let no_body = &[100, 101, 199, 204, 304];
    for status in no_body {
        assert!(!StatusCode(*status).includes_body());
    }
    let has_body = &[0, 10, 200, 201, 203, 205, 300, 301, 303, 305, 400, 500, 999];
    for status in has_body {
        assert!(StatusCode(*status).includes_body());
    }
}

#[test]
fn phrase() {
    #[rustfmt::skip]
    let tests = &[
        // 1xx range.
        (StatusCode::CONTINUE, "Continue"),
        (StatusCode::SWITCHING_PROTOCOLS, "Switching Protocols"),
        (StatusCode::PROCESSING, "Processing"),
        (StatusCode::EARLY_HINTS, "Early Hints"),

        // 2xx range.
        (StatusCode::OK, "OK"),
        (StatusCode::CREATED, "Created"),
        (StatusCode::ACCEPTED, "Accepted"),
        (StatusCode::NON_AUTHORITATIVE_INFORMATION, "Non-Authoritative Information"),
        (StatusCode::NO_CONTENT, "No Content"),
        (StatusCode::RESET_CONTENT, "Reset Content"),
        (StatusCode::PARTIAL_CONTENT, "Partial Content"),
        (StatusCode::MULTI_STATUS, "Multi-Status"),
        (StatusCode::ALREADY_REPORTED, "Already Reported"),
        (StatusCode::IM_USED, "IM Used"),

        // 3xx range.
        (StatusCode::MULTIPLE_CHOICES, "Multiple Choices"),
        (StatusCode::MOVED_PERMANENTLY, "Moved Permanently"),
        (StatusCode::FOUND, "Found"),
        (StatusCode::SEE_OTHER, "See Other"),
        (StatusCode::NOT_MODIFIED, "Not Modified"),
        (StatusCode::USE_PROXY, "Use Proxy"),
        (StatusCode::TEMPORARY_REDIRECT, "Temporary Redirect"),
        (StatusCode::PERMANENT_REDIRECT, "Permanent Redirect"),

        // 4xx range.
        (StatusCode::BAD_REQUEST, "Bad Request"),
        (StatusCode::UNAUTHORIZED, "Unauthorized"),
        (StatusCode::PAYMENT_REQUIRED, "Payment Required"),
        (StatusCode::FORBIDDEN, "Forbidden"),
        (StatusCode::NOT_FOUND, "Not Found"),
        (StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed"),
        (StatusCode::NOT_ACCEPTABLE, "Not Acceptable"),
        (StatusCode::PROXY_AUTHENTICATION_REQUIRED, "Proxy Authentication Required"),
        (StatusCode::REQUEST_TIMEOUT, "Request Timeout"),
        (StatusCode::CONFLICT, "Conflict"),
        (StatusCode::GONE, "Gone"),
        (StatusCode::LENGTH_REQUIRED, "Length Required"),
        (StatusCode::PRECONDITION_FAILED, "Precondition Failed"),
        (StatusCode::PAYLOAD_TOO_LARGE, "Payload Too Large"),
        (StatusCode::URI_TOO_LONG, "URI Too Long"),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Unsupported Media Type"),
        (StatusCode::RANGE_NOT_SATISFIABLE, "Range Not Satisfiable"),
        (StatusCode::EXPECTATION_FAILED, "Expectation Failed"),
        (StatusCode::MISDIRECTED_REQUEST, "Misdirected Request"),
        (StatusCode::UNPROCESSABLE_ENTITY, "Unprocessable Entity"),
        (StatusCode::LOCKED, "Locked"),
        (StatusCode::FAILED_DEPENDENCY, "Failed Dependency"),
        (StatusCode::TOO_EARLY, "Too Early"),
        (StatusCode::UPGRADE_REQUIRED, "Upgrade Required"),
        (StatusCode::PRECONDITION_REQUIRED, "Precondition Required"),
        (StatusCode::TOO_MANY_REQUESTS, "Too Many Requests"),
        (StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE, "Request Header Fields Too Large"),
        (StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS, "Unavailable For Legal Reasons"),

        // 5xx range.
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"),
        (StatusCode::NOT_IMPLEMENTED, "Not Implemented"),
        (StatusCode::BAD_GATEWAY, "Bad Gateway"),
        (StatusCode::SERVICE_UNAVAILABLE, "Service Unavailable"),
        (StatusCode::GATEWAY_TIMEOUT, "Gateway Timeout"),
        ( StatusCode::HTTP_VERSION_NOT_SUPPORTED, "HTTP Version Not Supported"),
        ( StatusCode::VARIANT_ALSO_NEGOTIATES, "Variant Also Negotiates"),
        (StatusCode::INSUFFICIENT_STORAGE, "Insufficient Storage"),
        (StatusCode::LOOP_DETECTED, "Loop Detected"),
        (StatusCode::NOT_EXTENDED, "Not Extended"),
        ( StatusCode::NETWORK_AUTHENTICATION_REQUIRED, "Network Authentication Required"),
    ];
    for (input, expected) in tests {
        assert_eq!(input.phrase().unwrap(), *expected);
    }

    assert!(StatusCode(0).phrase().is_none());
    assert!(StatusCode(999).phrase().is_none());
}

#[test]
fn cmp_with_u16() {
    let tests = &[
        (StatusCode::OK, 200, true),
        (StatusCode::BAD_REQUEST, 400, true),
        (StatusCode(999), 999, true),
        (StatusCode::OK, 201, false),
        (StatusCode::BAD_REQUEST, 399, false),
        (StatusCode(12), 999, false),
    ];
    for (status_code, n, expected) in tests {
        assert_eq!(status_code.eq(n), *expected);
    }
}

#[test]
fn fmt_display() {
    let tests = &[
        (StatusCode::OK, "200"),
        (StatusCode::BAD_REQUEST, "400"),
        (StatusCode(999), "999"),
    ];
    for (status_code, expected) in tests {
        assert_eq!(*status_code.to_string(), **expected);
    }
}
