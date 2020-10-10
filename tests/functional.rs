//! Functional tests.

#![feature(never_type, maybe_uninit_slice)]

#[cfg(not(feature = "test"))]
compile_error!(
    "Testing requires the `test` feature, enabled by adding `--features test` as argument"
);

mod util;

mod functional {
    mod bytes;
    mod from_message;
    mod restart_supervisor;
    mod tcp;
}
