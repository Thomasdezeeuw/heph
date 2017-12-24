// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

use std::io;

use futures::{Async, IntoFuture, Future};
use tchannel::mpsc;
use mio::Registration;
use mio::event::Evented;

use super::{ActorRef, Runner, RunnerId, RunnerType};
use super::super::actor::{Actor, NewActor};
use super::super::listener::NewListener;

pub struct ActorRunner<A: Actor> {
    actor: A,
    queue: mpsc::Receiver<A::Message>,
    registration: Registration,
    current: Option<<A::Future as IntoFuture>::Future>,
}

impl<A: Actor> ActorRunner<A> {
    /// Create new actor runner.
    pub fn new(actor: A, id: RunnerId) -> (ActorRunner<A>, ActorRef<A::Message>) {
        let (sender, receiver) = mpsc::channel();
        let (registration, set_readiness) = Registration::new2();
        let data = ActorRunner {
            actor,
            queue: receiver,
            registration,
            current: None,
        };
        let actor_ref = ActorRef { id, sender, set_readiness };
        (data, actor_ref)
    }
}

impl<A: Actor> Runner for ActorRunner<A> {
    fn runner_type(&self) -> RunnerType {
        RunnerType::Actor
    }

    fn start(&mut self) -> &Evented {
        self.actor.pre_start();
        &self.registration
    }

    fn run(&mut self) {
        // Try to receive another message if no future is in progress.
        if self.current.is_none() {
            let msg = match self.queue.try_receive() {
                Ok(msg) => msg,
                Err(_) => return, // No value, no need to run.
            };

            self.current = Some(self.actor.handle(msg).into_future());
        };

            Ok(Async::Ready(())) => self.run(),
            Ok(Async::NotReady) => self.current = Some(future),
        match self.current.as_mut().unwrap().poll() {
            Err(_) => {
                // TODO: send error to the supervisor.
                error!("error in handling of the actor");
            },
        }
    }

    fn stop(&mut self) {
        self.actor.post_stop();
    }
}

pub struct ListenerRunner<L: NewListener> {
    //new_listener: L,
    listener: L::Listener,
}

impl<L: NewListener> ListenerRunner<L> {
    /// Create new listener runner.
    pub fn new(new_listener: L) -> io::Result<ListenerRunner<L>> {
        let listener = new_listener.new()?;
        Ok(ListenerRunner { listener })
    }
}

impl<N: NewListener> Runner for ListenerRunner<N> {
    fn runner_type(&self) -> RunnerType {
        RunnerType::Listener
    }

    fn start(&mut self) -> &Evented {
        &self.listener
    }

    fn run(&mut self) {
        unimplemented!();
        // TODO:
        // 1. call accept
        // 2. if it has an item, handle it (by sending it to an actor?).
    }

    fn stop(&mut self) {
        // Do nothing.
    }
}
