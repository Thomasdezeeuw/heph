# Heph

Heph, derived from [Hephaestus], is the Greek god of blacksmiths, metalworking,
carpenters, craftsmen, artisans, sculptors, metallurgy, fire, and volcanoes.
Well this crate has very little to do with Greek gods, but I needed a name.

Quick links:

| Crate       | [crates.io](https://crates.io)                      | [docs.rs](https://docs.rs)                 |
| ----------- | --------------------------------------------------- | ------------------------------------------ |
| Heph        | [heph](https://crates.io/crates/heph)               | [heph](https://docs.rs/heph)               |
| Heph-rt     | [heph-rt](https://crates.io/crates/heph-rt)         | [heph-rt](https://docs.rs/heph-rt)         |
| Heph-http   | [heph-http](https://crates.io/crates/heph-http)     | [heph-http](https://docs.rs/heph-http)     |
| Heph-remote | [heph-remote](https://crates.io/crates/heph-remote) | [heph-remote](https://docs.rs/heph-remote) |
| Heph-inbox  | [heph-inbox](https://crates.io/crates/heph-inbox)   | [heph-inbox](https://docs.rs/heph-inbox)   |

[Hephaestus]: https://en.wikipedia.org/wiki/Hephaestus


## About

Heph is an [actor] framework based on asynchronous functions. Such an
asynchronous function looks like this:

```rust
async fn actor(mut ctx: actor::Context<String, ThreadLocal>) {
    // Receive a message.
    let msg = ctx.receive_next().await;
    // Print the message.
    println!("got a message: {msg}");
}
```

For more examples see the [examples] directory.

[actor]: https://en.wikipedia.org/wiki/Actor_model
[examples]: ./examples/README.md


## Design

Heph uses event-driven scheduling, non-blocking I/O (utiling io\_uring) and a
share nothing design. But what do all those buzzwords actually mean?

 - *Event-driven*: Heph does nothing by itself, it must first get an event
   before it starts doing anything. For example an actor is only run when it
   receives a message or I/O has been completed.
 - *Non-blocking I/O*: normal I/O operations need to wait (block) until the
   operation can complete. Using non-blocking, or asynchronous, I/O means that
   rather than waiting for the operation to complete we'll do some other more
   useful work and try the operation later. Furthermore using io\_uring we don't
   even have to make system calls (to do I/O) anymore!
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

Second, Heph needs to be added as a dependency. Most likely you'll also want the
Heph runtime (`heph-rt`).

```toml
[dependencies]
heph    = "0.5.0"
heph-rt = "0.5.0"
```

Now, you're ready to starting writing your application! Next, you can look at
some [examples], look at the [API documentation] or get started with the [Quick
Start Guide].

[rustup]: https://rustup.rs
[API documentation]: https://docs.rs/heph
[Quick Start Guide]:  https://docs.rs/heph/latest/heph/quick_start/index.html


## Platform support

~~The main target platform is Linux, as a production target. We also support
macOS, but only as development target (e.g. develop on macOS and run Linux in
production). Other BSDs are mostly supported (kqueue is fully supported),
however no tests are run on these platforms.~~

Since the switch to io\_uring only Linux is supported. Best supported is Linux
TLS versions, at the time of writing Linux v6.1. Support for other Unix targets
such as macOS and the BSDs might return in the future.


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
system (part of the standard library), asynchronous functions, and [A10].

[A10]: https://github.com/Thomasdezeeuw/a10
