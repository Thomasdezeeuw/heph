//! Regression tests.

use heph_inbox::oneshot::{self, new_oneshot};
use heph_inbox::{self as inbox, new};

#[macro_use]
mod util;

#[test]
fn cyclic_drop_dependency_with_oneshot_channel() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new(capacity);
        let (one_send, mut one_recv) = new_oneshot::<usize>();

        // Put `oneshot::Sender` in the channel.
        sender.try_send(one_send).unwrap();

        // Dropping the receiver should also drop the `oneshot::Sender` we send
        // above.
        drop(receiver);
        // This needs to work in case we would call `oneshot::Receiver::recv` here.
        // If we didn't empty the channel on dropping the `Receiver` this would
        // return a `NoValue` error.
        assert_eq!(
            one_recv.try_recv().unwrap_err(),
            oneshot::RecvError::Disconnected
        );

        drop(one_recv);
        // This needs to live until now.
        drop(sender);
    });
}
