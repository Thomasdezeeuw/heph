//! Regression tests.
//!
//! Format is `tests/regression/issue_#.rs`, referring to a GitHub issue #.

#![feature(never_type)]

#[path = "regression"] // rustfmt can't find the files.
mod regression {
    mod issue_145;
    mod issue_294;
    mod issue_323;
}
