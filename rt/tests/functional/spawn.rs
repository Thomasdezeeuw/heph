//! Tests to check what types implement the Spawn trait.

use std::future::Pending;
use std::marker::PhantomData;

use heph::actor::{self, NewActor};
use heph::supervisor::NoSupervisor;
use heph_rt::spawn::Spawn;
use heph_rt::{Runtime, RuntimeRef, ThreadLocal, ThreadSafe};

struct TestNewActor<RT>(PhantomData<RT>);

impl<RT> NewActor for TestNewActor<RT> {
    type Message = !;
    type Argument = ();
    type Actor = Pending<Result<(), !>>;
    type Error = !;
    type RuntimeAccess = RT;

    fn new(
        &mut self,
        _: actor::Context<Self::Message, Self::RuntimeAccess>,
        _: Self::Argument,
    ) -> Result<Self::Actor, Self::Error> {
        todo!()
    }
}

fn can_spawn_thread_local<T>()
where
    T: Spawn<NoSupervisor, TestNewActor<ThreadLocal>, ThreadLocal>,
{
}

fn can_spawn_thread_safe<T>()
where
    T: Spawn<NoSupervisor, TestNewActor<ThreadSafe>, ThreadSafe>,
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
