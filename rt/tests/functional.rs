//! Functional tests.

#![feature(async_iterator, never_type, write_all_vectored)]

#[path = "util/mod.rs"] // rustfmt can't find the file.
#[macro_use]
mod util;

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod access;
    mod actor;
    mod actor_context;
    mod actor_group;
    mod actor_ref;
    mod from_message;
    mod fs;
    mod fs_watch;
    mod future;
    mod io;
    mod pipe;
    mod restart_supervisor;
    mod runtime;
    mod signal;
    mod spawn;
    mod sync_actor;
    mod tcp;
    mod test;
    mod timer;
    mod udp;
    mod uds;
}
