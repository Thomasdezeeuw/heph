//! Tests for the access module.

use heph_rt::{Runtime, RuntimeRef, ThreadLocal, ThreadSafe};

use crate::util::{assert_send, assert_sync};

#[test]
fn thread_safe_is_send_sync() {
    assert_send::<ThreadSafe>();
    assert_sync::<ThreadSafe>();
}

#[test]
fn thread_local_from_runtime_ref() {
    let mut rt = Runtime::new().unwrap();
    rt.run_on_workers(|runtime_ref| {
        let thread_local = ThreadLocal::from(runtime_ref);
        drop(thread_local);
        Ok::<_, !>(())
    })
    .unwrap();
}

#[test]
fn thread_local_deref_as_runtime_ref() {
    let mut rt = Runtime::new().unwrap();
    rt.run_on_workers(|runtime_ref| {
        let mut thread_local = ThreadLocal::from(runtime_ref);
        let _runtime_ref: &mut RuntimeRef = &mut *thread_local;
        drop(thread_local);
        Ok::<_, !>(())
    })
    .unwrap();
}

#[test]
fn thread_safe_from_runtime() {
    let rt = Runtime::new().unwrap();
    let thread_safe = ThreadSafe::from(&rt);
    drop(thread_safe);
    drop(rt);
}

#[test]
fn thread_safe_from_runtime_ref() {
    let mut rt = Runtime::new().unwrap();
    rt.run_on_workers(|runtime_ref| {
        let thread_safe = ThreadSafe::from(&runtime_ref);
        drop(thread_safe);
        Ok::<_, !>(())
    })
    .unwrap();
}
