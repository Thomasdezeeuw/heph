# Heph

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Crates.io](https://img.shields.io/crates/v/heph.svg)](https://crates.io/crates/heph)
[![Docs](https://docs.rs/heph/badge.svg)](https://docs.rs/heph)

Heph, derived from [Hephaestus], is the Greek god of blacksmiths, metalworking,
carpenters, craftsmen, artisans, sculptors, metallurgy, fire, and volcanoes.
Well this crate has very little to do with Greek gods, but I needed a name.

[Hephaestus]: https://en.wikipedia.org/wiki/Hephaestus


## About

Heph is an [actor] framework based on asynchronous functions. Such an
asynchronous function looks like this:

```rust
async fn actor(mut ctx: actor::Context<String, ThreadLocal>) {
    // Receive a message.
    let msg = ctx.receive_next().await;
    // Print the message.
    println!("got a message: {}", msg);
}
```

For more examples see the [examples] directory.

[actor]: https://en.wikipedia.org/wiki/Actor_model
[examples]: ./examples/README.md


## Design

Heph uses an event-driven, non-blocking I/O, share nothing design. But what do
all those buzzwords actually mean?

 - *Event-driven*: Heph does nothing by itself, it must first get an event
   before it starts doing anything. For example when using a `TcpListener` it
   waits on a notification from the OS saying the `TcpListener` is ready before
   trying to accepted connections.
 - *Non-blocking I/O*: normal I/O operations need to wait (block) until the
   operation can complete. Using non-blocking, or asynchronous, I/O means that
   rather than waiting for the operation to complete we'll do some other more
   useful work and try the operation later.
 - *Share nothing*: many applications share data across multiple threads. To
   do this safely, we need to protect it from data races via a [`Mutex`] or
   by using [atomic] operations. Heph is designed to not share any data. Each
   actor is responsible for its own memory and cannot access memory owned by
   other actors. Instead, communication is done via sending messages–see [actor
   model].

[`Mutex`]: https://doc.rust-lang.org/std/sync/struct.Mutex.html
[atomic]: https://doc.rust-lang.org/std/sync/atomic/index.html
[actor model]: https://en.wikipedia.org/wiki/Actor_model


## Getting started

First, you'll need a recent nightly compiler. [rustup] is the easiest way
to install and manage different rust installations. The following command will
install a nightly compiler with rustup.

```bash
rustup install nightly # Install the latest nightly compiler.

# Optional:
rustup default nightly # Set the nightly compiler as default.
```

Second, Heph needs to be added as a dependency.

```toml
[dependencies]
heph = "0.3.0"
```

Now, you're ready to starting writing your application! Next, you can look at
some [examples] or look at the [API documentation].

[rustup]: https://rustup.rs
[API documentation]: https://docs.rs/heph


## Platform support

The main target platform is Linux, as a production target. We also support
macOS, but only as development target (e.g. develop on macOS and run Linux in
production). Other BSDs are mostly supported (kqueue is fully supported),
however no tests are run on these platforms.


## Stability

Currently this project is *unstable*, since the crate depends on many
experimental or Nightly only Rust features. So, it can only be compiled using a
Nightly version of the Rust compiler. Furthermore, the crate itself is
< v1, meaning the API is far from stable as well. In fact, improvements to the
API are very welcome!


## License

Licensed under the MIT license ([LICENSE] or
https://opensource.org/licenses/MIT).

[LICENSE]: ./LICENSE


### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you shall be licensed as above, without any
additional terms or conditions.


## Inspiration

Heph is inspired by a number of other frameworks and languages. The greatest
inspiration is the [Erlang] programming language. From Erlang, the concept of
processes, or rather a function as a process, is borrowed. In line with the
actor model, the processes (actors) cannot access each other's memory, instead
message passing is used to communicate.

Another inspiration is the [Akka] framework for Scala and Java. Where Erlang is
a functional language, Akka is implemented in/for Scala and Java, languages more
in line with Rust (both being object oriented, but having functional aspects). A
lot of the API is inspired by the API provided in Akka, but there are a number
of big differences. The main one being that Akka's actor are untyped (or at
least can be), where Heph actors are statically typed (just like Rust in 
general).

The final inspiration I would like to mention is [Nginx]. Nginx is an HTTP and
reverse proxy server. The architecture used in Nginx is running a single master
process (not a thread) that coordinates a number of worker processes (again not
threads), each with their own polling instance (epoll/kqueue etc.) which all
share the same TCP listeners. In early stages of development of Heph, the idea
was to start a new process per CPU core (much like Nginx). This would allow all
atomic and locking operations to be dropped. However, this was later changed to
start threads instead to reduce the complexity when working with other
libraries. In the original design, it wouldn't be possible to work with any
libraries that started threads because we could introduce data races–something
that Rust tries very hard to avoid. Still, most of the architecture was
inspired by the one used in Nginx.

[Erlang]: https://www.erlang.org
[Akka]: https://akka.io
[Nginx]: https://nginx.org


### Building blocks

Besides the inspiration gained from the work of others, Heph is also built upon
the work of others. Three major components used in Heph are the futures task
system (part of the standard library), asynchronous functions, and [Mio].

[Mio]: https://github.com/tokio-rs/mio
