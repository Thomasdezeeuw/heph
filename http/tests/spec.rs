//! Specification compliance tests.

#![feature(never_type, once_cell)]

#[path = "spec"] // rustfmt can't find the files.
mod spec {
    mod rfc7230;
}
