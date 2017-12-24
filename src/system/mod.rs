// Copyright 2017 Thomas de Zeeuw
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT
// or http://opensource.org/licenses/MIT>, at your option. This file may not be
// used, copied, modified, or distributed except according to those terms.

use std::{fmt, io};

use fnv::FnvHashMap;
use mio::{Token, Ready, SetReadiness, Poll, PollOpt};
use tchannel::mpsc;
use mio::event::{Events, Evented};

use super::actor::{Actor, NewActor};
use super::listener::NewListener;

mod builder;
mod runners;

use self::runners::{ActorRunner, ListenerRunner};

pub use self::builder::ActorSystemBuilder;

/// This is an important internal trait that defines how an actor, or listener,
/// is run.
///
/// When the system is initalized all the actors and listeners will be added to
/// the system. After that the system can be started, which loops over all
/// actors and listeners and starts them.
trait Runner {
    /// Returns the runner type.
    // TODO: this is not the most elegant of APIs, try to remove it.
    fn runner_type(&self) -> RunnerType;

    /// Start a runner, returning a reference to it's registration.
    fn start(&mut self) -> &Evented;

    /// Run the runner. For listeners this will check if it can accept another
    /// item, for actors it will see if it can processes another message.
    ///
    /// This will only be called if the registration of the runner is notified.
    fn run(&mut self);

    /// Stop the runner from further processing.
    // TODO: return an future that needed to be waited on? Or return a enum
    // indicates shutdown or not.
    fn stop(&mut self);
}

/// The type of runner. If not Listener runner are present in the system it
/// doesn't run the event loop for ever.
enum RunnerType {
    Listener,
    Actor,
}

/// Internal id of a `Runner`.
///
/// This unique in the system and is used in the registration with mio.
type RunnerId = usize;

pub struct ActorSystem<'a> {
    current_id: RunnerId,
    // TODO: replace this with a concurrent (hash)map, so the add_actor method
    // can become reference only.
    runners: FnvHashMap<RunnerId, Box<Runner + 'a>>,
}

impl<'a> ActorSystem<'a> {
    /// Add a new actor to the system.
    ///
    /// This returns a reference to the actor, which can be used to send
    /// messages to it.
    // TODO: add supervisor to handle returned errors?
    pub fn add_actor<A>(&mut self, actor: A) -> ActorRef<A::Message>
        where A: Actor + 'a,
    {
        let id = self.new_id();
        let (actor_runner, actor_ref) = ActorRunner::new(actor, id);
        self.runners.insert(id, Box::new(actor_runner));
        actor_ref
    }

    /// Add a new listener to the system.
    pub fn add_listener<N>(&mut self, new_listener: N) -> io::Result<()>
        where N: NewListener + 'a,
    {
        let id = self.new_id();
        let listener_runner = ListenerRunner::new(new_listener)?;
        self.runners.insert(id, Box::new(listener_runner));
        Ok(())
    }

    /// Generate a new unique `RunnerId`.
    fn new_id(&mut self) -> RunnerId {
        let id = self.current_id;
        self.current_id += 1;
        id
    }

    pub fn run(mut self) -> io::Result<()> {
        // TODO: setup a interrupt handler.
        // TODO: rewrite to use multiple threads.

        trace!("starting up");
        let poll = self.register_runners()?;

        if !self.has_listeners() {
            // FIXME: fix this, we need a good way to detect that the system is
            // done.
            error!("actor system detected no listeners, system will be waiting \
                for ever after initial message passing sequence");
        }

        // FIXME: setup Future environment.

        trace!("running event loop");
        let mut events = Events::with_capacity(1024);
        loop {
            trace!("polling for events");
            poll.poll(&mut events, None)?;

            for event in events.iter() {
                trace!("got event: {:?}, {:?}", event.token(), event.readiness());
                let runner_id = event.token().into();
                match self.runners.get_mut(&runner_id) {
                    Some(runner) => {
                        trace!("running runner {}", runner_id);
                        runner.run();
                    },
                    None => warn!("got event with unknown token: {:?}", event),
                }
            }
        }
    }

    fn register_runners(&mut self) -> io::Result<Poll> {
        let poll = Poll::new()?;
        for (id, runner) in self.runners.iter_mut() {
            trace!("registering runner {}", id);
            let token = Token(*id);
            let evented = runner.start();
            poll.register(evented, token, Ready::readable() | Ready::writable(),
                PollOpt::edge())?;
        }
        Ok(poll)
    }

    fn has_listeners(&self) -> bool {
        for runner in self.runners.values() {
            if let RunnerType::Listener = runner.runner_type() {
                return true
            }
        }
        false
    }
}

impl<'a> Default for ActorSystem<'a> {
    fn default() -> ActorSystem<'a> {
        ActorSystemBuilder::default().build()
    }
}

/// A reference to an actor, used to send messages to it.
///
/// To share this reference simply clone it.
// TODO: add example on how to share the reference and how to send messages.
pub struct ActorRef<M> {
    id: RunnerId,
    sender: mpsc::Sender<M>,
    set_readiness: SetReadiness,
}

impl<M> ActorRef<M> {
    /// Send a message to the actor.
    ///
    /// # Panic
    ///
    /// This will panic if the system is shutting down.
    pub fn send(&mut self, msg: M) {
        const PANIC_MSG: &'static str = "unable to send to actor, is the system shutting down?";
        self.sender.send(msg)
            .unwrap_or_else(|_| panic!(PANIC_MSG));
        // FIXME: fix memory ordering. This may not be used concurrently.
        self.set_readiness.set_readiness(Ready::readable())
            .unwrap_or_else(|_| panic!(PANIC_MSG));
    }
}

impl<M> fmt::Debug for ActorRef<M> {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("ActorRef")
            .field("id", &self.id)
            .finish()
    }
}

/* FIXME: SetReadiness is not concurrency safe, fix that first.
impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> ActorRef<M> {
        ActorRef {
            id: self.id,
            sender: self.sender.clone(),
            set_readiness: self.set_readiness.clone(),
        }
    }
}
*/
