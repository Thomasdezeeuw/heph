//! Functional tests.

#![feature(async_iterator, never_type)]

mod util;

#[path = "functional/access.rs"]
mod access;
#[path = "functional/actor_context.rs"]
mod actor_context;
#[path = "functional/actor_group.rs"]
mod actor_group;
#[path = "functional/actor_ref.rs"]
mod actor_ref;
#[path = "functional/from_message.rs"]
mod from_message;
#[path = "functional/future.rs"]
mod future;
#[path = "functional/pipe.rs"]
mod pipe;
#[path = "functional/restart_supervisor.rs"]
mod restart_supervisor;
#[path = "functional/runtime.rs"]
mod runtime;
#[path = "functional/scheduler/mod.rs"]
mod scheduler;
#[path = "functional/spawn.rs"]
mod spawn;
#[path = "functional/sync_actor.rs"]
mod sync_actor;
#[path = "functional/tcp_server.rs"]
mod tcp_server;
#[path = "functional/test.rs"]
mod test;
#[path = "functional/timer.rs"]
mod timer;
