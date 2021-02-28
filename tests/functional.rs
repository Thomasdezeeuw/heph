//! Functional tests.

#![feature(
    async_stream,
    drain_filter,
    maybe_uninit_slice,
    never_type,
    write_all_vectored
)]

#[path = "util/mod.rs"] // rustfmt can't find the file.
#[macro_use]
mod util;

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod actor;
    mod actor_context;
    mod actor_group;
    mod actor_ref;
    mod bytes;
    mod from_message;
    mod restart_supervisor;
    mod runtime;
    mod sync_actor;
    mod tcp;
    mod test;
    mod timer;
    mod udp;
}
