//! Functional tests.

#![feature(never_type, maybe_uninit_slice)]

mod util;

mod functional {
    mod actor_ref;
    mod bytes;
    mod from_message;
    mod restart_supervisor;
    mod sync_actor;
    mod tcp;
    mod udp;
}
