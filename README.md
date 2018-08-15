# Heph

Heph derived from Hephaestus, is the Greek god of blacksmiths, metalworking,
carpenters, craftsmen, artisans, sculptors, metallurgy, fire, and volcanoes.
<sup>[1]</sup> Well this crate has very little to do with Greek gods, but I
needed a name.


## About

Heph is an actor <sup>[2]</sup> framework based on asynchronous functions. Such
an asynchronous function looks like this:

```rust
async fn print_actor(mut ctx: ActorContext<String>, _: ()) -> Result<(), !> {
    // Receive a message.
    let msg = await!(ctx.receive());
    // Print the message.
    println!("got a message: {}", msg);
    // And we're done.
    Ok(())
}
```

For more examples see the [examples] directory.

[examples]: ./examples/README.md


## Design

Heph uses an event-driven, non-blocking I/O, share nothing design. But what do
all those buzzwords actually mean?

 - *Event-driven*: Heph does nothing by itself, it must first get an event
   before it starts doing anything. For example when using an `TcpListener` it
   waits on a notification from the OS saying the `TcpListener` is ready before
   trying to accepted connections.
 - *Non-blocking I/O*: normal I/O operations need to wait (block) until the
   operation can complete. Using non-blocking, or asynchronous, I/O means that
   rather then waiting for the operation to complete we'll do some other more
   useful work and try the operation later.
 - *Share nothing*: a lot of application share data across multiple threads. To
   do this safely we need to protect it from data races, via a [`Mutex`] or
   by using [atomic] operations. Heph is designed to not share any data. Each
   actor is responsible for its own memory and cannot access memory owned by
   other actors. Instead communication is done via sending messages, see actor
   model. <sup>[2]</sup>

[`Mutex`]: https://doc.rust-lang.org/std/sync/struct.Mutex.html
[atomic]: https://doc.rust-lang.org/std/sync/atomic/index.html


## Getting started

First you'll need a recent nightly compiler. The easiest way to install and
manage different rust installations is via [rustup]. The following command will
install a nightly compiler via rustup.

```bash
rustup install nightly # Install the latests nightly compiler.

# Optional:
rustup default nightly # Set the nightly compiler as default.
```

Second Heph needs to be added as a dependency.

```toml
[dependencies]
heph = "0.1.0"
```

If your using the 2015 edition of Rust (the default) you need to add an `extern
crate` statement to your code.

```rust
extern crate heph;
```

Now you're ready to starting writing your application! Next you can look at some
[examples], or look at the [API documentation].

[rust-toolchain]: rust-toolchain
[rustup]: https://rustup.rs
[API documentation]: https://docs.rs/heph


## License

Licensed under the MIT license ([LICENSE] or
https://opensource.org/licenses/MIT).

[LICENSE]: ./LICENSE


### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
licensed as above, without any additional terms or conditions.


[1]: https://en.wikipedia.org/wiki/Hephaestus
[2]: https://en.wikipedia.org/wiki/Actor_model
