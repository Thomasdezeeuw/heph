use std::borrow::Cow;

// TODO: support streaming bodies.
pub trait Body {
    fn as_bytes(&self) -> &[u8];
}

impl Body for [u8] {
    fn as_bytes(&self) -> &[u8] {
        self
    }
}

impl Body for Vec<u8> {
    fn as_bytes(&self) -> &[u8] {
        &*self
    }
}

impl Body for Cow<'_, [u8]> {
    fn as_bytes(&self) -> &[u8] {
        self.as_ref()
    }
}

impl Body for str {
    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Body for String {
    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Body for Cow<'_, str> {
    fn as_bytes(&self) -> &[u8] {
        self.as_ref().as_bytes()
    }
}
