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

// TODO: reduce the internal state for both runners.

pub struct ActorRunner<N: NewActor> {
    //new_actor: N,
    actor: N::Actor,
    queue: mpsc::Receiver<<N::Actor as Actor>::Message>,
    registration: Registration,
    // lovely type signature.
    current: Option<<<N::Actor as Actor>::Future as IntoFuture>::Future>,
}

impl<N: NewActor> ActorRunner<N> {
    /// Create new actor runner.
    pub fn new(new_actor: N, id: RunnerId) -> (ActorRunner<N>, ActorRef<<N::Actor as Actor>::Message>) {
        let (sender, receiver) = mpsc::channel();
        let (registration, set_readiness) = Registration::new2();
        let actor = new_actor.new();
        let data = ActorRunner {
            //new_actor,
            actor,
            queue: receiver,
            registration,
            current: None,
        };
        let actor_ref = ActorRef { id, sender, set_readiness };
        (data, actor_ref)
    }
}

impl<N: NewActor> Runner for ActorRunner<N> {
    fn runner_type(&self) -> RunnerType {
        RunnerType::Actor
    }

    fn start(&mut self) -> &Evented {
        self.actor.pre_start();
        &self.registration
    }

    fn run(&mut self) {
        // Either grap the in progress future or try to receive a new message
        // from the channel.
        let mut future = if self.current.is_some() {
            self.current.take().unwrap()
        } else {
            let msg = match self.queue.try_receive() {
                Ok(msg) => msg,
                Err(_) => return, // No value, no need to run.
            };

            self.actor.handle(msg).into_future()
        };

        match future.poll() {
            Ok(Async::Ready(())) => self.run(),
            Ok(Async::NotReady) => self.current = Some(future),
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
