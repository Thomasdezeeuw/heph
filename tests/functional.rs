//! Functional tests.

#![feature(never_type)]

mod util {
    use std::mem::size_of;

    pub fn assert_send<T: Send>() {}

    pub fn assert_sync<T: Sync>() {}

    #[track_caller]
    pub fn assert_size<T>(expected: usize) {
        assert_eq!(size_of::<T>(), expected);
    }
}

#[path = "functional"] // rustfmt can't find the files.
mod functional {
    mod actor;
    mod actor_group;
    mod actor_ref;
    mod restart_supervisor;
    mod sync_actor;
    mod test;
}
