# Inbox

Bounded capacity channel.

The channel is a multi-producer, single-consumer (MPSC) bounded queue. It is
designed to be used as inbox for actors, following the [actor model].

[actor model]: https://en.wikipedia.org/wiki/Actor_model

# Examples

Simple creation of a channel and sending a message over it.

```rust
use std::thread;

use heph_inbox::RecvError;

// Create a new small channel.
let (mut sender, mut receiver) = heph_inbox::new_small();

let sender_handle = thread::spawn(move || {
    if let Err(err) = sender.try_send("Hello world!".to_owned()) {
        panic!("Failed to send value: {}", err);
    }
});

let receiver_handle = thread::spawn(move || {
    // NOTE: this is just an example don't actually use a loop like this, it
    // will waste CPU cycles when the channel is empty!
    loop {
        match receiver.try_recv() {
            Ok(value) => println!("Got a value: {}", value),
            Err(RecvError::Empty) => continue,
            Err(RecvError::Disconnected) => break,
        }
    }
});

sender_handle.join().unwrap();
receiver_handle.join().unwrap();
```

## License

Licensed under the MIT license ([LICENSE](LICENSE) or
https://opensource.org/licenses/MIT).

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you shall be licensed as above, without any
additional terms or conditions.
