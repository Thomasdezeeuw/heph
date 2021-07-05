use heph_http::Url;

use crate::assert_size;

#[test]
fn size() {
    assert_size::<Url>(88);
}

/*
use heph_http::Uri;

use crate::assert_size;

#[test]
#[ignore = "TODO"]
fn size() {
    assert_size::<Uri>(1);
}

#[test]
fn uri_parts() {
    let tests = &[
        // origin-form
        (
            "/some/path/to/something",
            None,
            None,
            None,
            None,
            "/some/path/to/something",
            None,
        ),
        (
            "/path/with/a/query?some_query=value&other=value2",
            None,
            None,
            None,
            None,
            "/path/with/a/query",
            Some("some_query=value&other=value2"),
        ),
        // absolute-form.
        /*
        (
            "http://www.example.org/pub/WWW/TheProject.html",
            Some("http"),
            Some("www.example.org"),
            Some("www.example.org"),
            None,
            "/pub/WWW/TheProject.html",
            None,
        ),
        (
            "https://www.example.org:8080/pub/WWW/TheProject.html?some_query=value&other=value2",
            Some("https"),
            Some("www.example.org:8080"),
            Some("www.example.org"),
            Some(8080),
            "/pub/WWW/TheProject.html",
            Some("some_query=value&other=value2"),
        ),
        (
            "http://127.0.0.1/pub/WWW/TheProject.html",
            Some("http"),
            Some("127.0.0.1"),
            Some("127.0.0.1"),
            None,
            "/pub/WWW/TheProject.html",
            None,
        ),
        (
            "http://127.0.0.1:8080/pub/WWW/TheProject.html",
            Some("http"),
            Some("127.0.0.1:8080"),
            Some("127.0.0.1"),
            None,
            "/pub/WWW/TheProject.html",
            None,
        ),
        */
        (
            "http://[2001:db8:85a3:8d3:1319:8a2e:370:7348]/",
            Some("http"),
            Some("[2001:db8:85a3:8d3:1319:8a2e:370:7348]"),
            Some("2001:db8:85a3:8d3:1319:8a2e:370:7348"),
            None,
            "/",
            None,
        ),
        (
            "https://[2001:db8:85a3:8d3:1319:8a2e:370:7348]:443/",
            Some("https"),
            Some("[2001:db8:85a3:8d3:1319:8a2e:370:7348]:443"),
            Some("2001:db8:85a3:8d3:1319:8a2e:370:7348"),
            Some(443),
            "/",
            None,
        ),
        // authority-form
        /* TODO.
        (
            "www.example.org",
            None,
            Some("www.example.org"),
            Some("www.example.org"),
            None,
            "",
            None,
        ),
        (
            "www.example.org:8080",
            None,
            Some("www.example.org:8080"),
            Some("www.example.org"),
            Some(8080),
            "",
            None,
        ),
        (
            "www.example.org:8080/abc",
            None,
            Some("www.example.org:8080"),
            Some("www.example.org"),
            Some(8080),
            "/abc",
            None,
        ),
        (
            "thomas@www.example.org:8080/abc",
            None,
            Some("www.example.org:8080"),
            Some("www.example.org"),
            Some(8080),
            "/abc",
            None,
        ),
        */
        // TODO: add userinfo tests.
        // asterisk-form.
        ("*", None, None, None, None, "*", None),
    ];

    for (uri, scheme, authority, host, port, path, query) in tests {
        dbg!(uri); // FIXME: remove.
        let uri = Uri::new(uri.to_string());
        assert_eq!(uri.scheme(), *scheme);
        assert_eq!(uri.authority(), *authority);
        assert_eq!(uri.host(), *host);
        assert_eq!(uri.port(), *port);
        assert_eq!(uri.path(), *path);
        assert_eq!(uri.query(), *query);
    }
}

#[test]
#[ignore = "fails"]
fn asterisk_form() {
    let tests = &[
        ("*", true),
        ("/", false),
        ("/some/path", false),
        ("/index.html", false),
    ];
    for (uri, expected) in tests {
        let uri = Uri::new(uri.to_string());
        assert_eq!(uri.is_asterisk(), *expected, "uri: {:?}", uri);
    }
}
*/
