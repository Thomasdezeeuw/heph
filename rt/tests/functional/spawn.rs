//! Tests to check what types implement the Spawn trait.

use heph::actor;
use heph_rt::spawn::{Spawn, SpawnLocal};
use heph_rt::{Runtime, RuntimeRef, Sync, ThreadLocal, ThreadSafe};

fn can_spawn_thread_local<T>()
where
    T: SpawnLocal,
{
}

fn can_spawn_thread_safe<T>()
where
    T: Spawn,
{
}

#[test]
fn runtime() {
    can_spawn_thread_safe::<Runtime>();
}

#[test]
fn runtime_ref() {
    can_spawn_thread_local::<RuntimeRef>();
    can_spawn_thread_safe::<RuntimeRef>();
}

#[test]
fn thread_local_actor_context() {
    can_spawn_thread_local::<actor::Context<(), ThreadLocal>>();
    can_spawn_thread_safe::<actor::Context<(), ThreadLocal>>();
}

#[test]
fn thread_safe_actor_context() {
    can_spawn_thread_safe::<actor::Context<(), ThreadSafe>>();
}

#[test]
fn thread_local() {
    can_spawn_thread_local::<ThreadLocal>();
    can_spawn_thread_safe::<ThreadLocal>();
}

#[test]
fn thread_safe() {
    can_spawn_thread_safe::<ThreadSafe>();
}

#[test]
fn sync() {
    can_spawn_thread_safe::<Sync>();
}
