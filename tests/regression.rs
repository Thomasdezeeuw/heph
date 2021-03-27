//! Regression tests.
//!
//! Format is `tests/regression/issue_#.rs`, referring to a GitHub issue #.

#![feature(never_type)]

#[cfg(not(feature = "test"))]
compile_error!("needs `test` feature, run with `cargo test --all-features`");

#[path = "regression"] // rustfmt can't find the files.
mod regression {
    mod issue_145;
    mod issue_294;
    mod issue_323;
}
