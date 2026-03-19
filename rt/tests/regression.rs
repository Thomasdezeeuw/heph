//! Regression tests.
//!
//! Format is `tests/regression/issue_#.rs`, referring to a GitHub issue #.

#![feature(never_type)]

#[path = "regression/issue_145.rs"]
mod issue_145;
#[path = "regression/issue_294.rs"]
mod issue_294;
#[path = "regression/issue_323.rs"]
mod issue_323;
