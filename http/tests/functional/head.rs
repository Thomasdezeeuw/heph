use heph_http::head::{RequestHead, ResponseHead};

use crate::assert_size;

#[test]
fn size() {
    assert_size::<RequestHead>(80);
    assert_size::<ResponseHead>(56);
}
