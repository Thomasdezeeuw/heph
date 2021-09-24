use heph_http::body::OneshotBody;

const BODY1: &[u8] = b"Hello world!";

#[test]
fn oneshot_bytes() {
    let body = OneshotBody::new(BODY1);
    assert_eq!(body.bytes(), BODY1);
}

#[test]
fn oneshot_cmp_bytes() {
    let body = OneshotBody::new(BODY1);
    assert_eq!(body, BODY1);
}

#[test]
fn oneshot_cmp_string() {
    let body = OneshotBody::new(BODY1);
    assert_eq!(body, "Hello world!");
}
