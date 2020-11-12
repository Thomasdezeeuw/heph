//! Functional tests.

#![feature(never_type, maybe_uninit_slice)]

mod util;

mod functional {
    mod bytes;
    mod from_message;
    mod restart_supervisor;
    mod tcp;
    mod udp;
}
