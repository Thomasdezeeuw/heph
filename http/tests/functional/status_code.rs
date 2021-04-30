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
