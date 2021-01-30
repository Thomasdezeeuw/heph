//! Functional tests.

#![feature(drain_filter, never_type, maybe_uninit_slice, write_all_vectored)]

#[path = "util/mod.rs"] // rustfmt can't find the file.
#[macro_use]
mod util;

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod actor_context;
    mod actor_group;
    mod actor_ref;
    mod bytes;
    mod from_message;
    mod restart_supervisor;
    mod runtime;
    mod sync_actor;
    mod tcp;
    mod timer;
    mod udp;
}
