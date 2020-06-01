//! Functional tests.

use inbox::{new_small, RecvError, SendError};

const N: usize = 8;

#[test]
fn sending_and_receiving_value() {
    let (mut sender, mut receiver) = new_small::<usize>();
    sender.try_send(123).unwrap();
    assert_eq!(receiver.try_recv().unwrap(), 123);
}

#[test]
fn receiving_from_empty_channel() {
    let (_sender, mut receiver) = new_small::<usize>();
    assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Empty);
}

#[test]
fn receiving_from_disconnected_channel() {
    let (sender, mut receiver) = new_small::<usize>();
    drop(sender);
    assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
}

#[test]
fn sending_into_full_channel() {
    let (mut sender, receiver) = new_small::<usize>();
    for value in 0..N {
        sender.try_send(value).unwrap();
    }
    assert_eq!(sender.try_send(N + 1), Err(SendError::Full(N + 1)));
    drop(receiver);
}

#[test]
fn send_len_values_send_then_recv() {
    let (mut sender, mut receiver) = new_small::<usize>();
    for value in 0..N {
        sender.try_send(value).unwrap();
    }
    assert!(sender.try_send(N + 1).is_err());
    for value in 0..N {
        assert_eq!(receiver.try_recv().unwrap(), value);
    }
    assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Empty);
}

#[test]
fn send_len_values_interleaved() {
    let (mut sender, mut receiver) = new_small::<usize>();
    for value in 0..N {
        sender.try_send(value).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), value);
    }
}

#[test]
fn send_2_len_values_send_then_recv() {
    let (mut sender, mut receiver) = new_small::<usize>();
    for value in 0..N {
        sender.try_send(value).unwrap();
    }
    for value in 0..N {
        assert_eq!(receiver.try_recv().unwrap(), value);
        sender.try_send(N + value).unwrap();
    }
    for value in 0..N {
        assert_eq!(receiver.try_recv().unwrap(), N + value);
    }
}

#[test]
fn send_2_len_values_interleaved() {
    let (mut sender, mut receiver) = new_small::<usize>();
    for value in 0..2 * N {
        sender.try_send(value).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), value);
    }
}

#[test]
fn sender_disconnected_after_send() {
    let (mut sender, mut receiver) = new_small::<usize>();
    sender.try_send(123).unwrap();
    drop(sender);
    assert_eq!(receiver.try_recv().unwrap(), 123);
    assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
}

#[test]
fn sender_disconnected_after_send_len() {
    let (mut sender, mut receiver) = new_small::<usize>();
    for value in 0..N {
        sender.try_send(value).unwrap();
    }
    drop(sender);
    for value in 0..N {
        assert_eq!(receiver.try_recv().unwrap(), value);
    }
    assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
}

#[test]
fn sender_disconnected_after_send_2_len() {
    let (mut sender, mut receiver) = new_small::<usize>();
    for value in 0..2 * N {
        sender.try_send(value).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), value);
    }
    drop(sender);
    assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
}

const LARGE: usize = 1_000_000;

#[test]
#[cfg_attr(not(feature = "stress_testing"), ignore)]
fn stress_sending_interleaved() {
    let (mut sender, mut receiver) = new_small::<usize>();
    for value in 0..LARGE {
        sender.try_send(value).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), value);
    }
    assert_eq!(receiver.try_recv(), Err(RecvError::Empty));
}

#[test]
#[cfg_attr(not(feature = "stress_testing"), ignore)]
fn stress_sending_fill() {
    for n in 1..=(N - 1) {
        let (mut sender, mut receiver) = new_small::<usize>();

        for value in 0..(LARGE / n) {
            for n in 0..n {
                sender.try_send(value + n).unwrap();
            }
            for n in 0..n {
                assert_eq!(receiver.try_recv().unwrap(), value + n);
            }
        }

        assert_eq!(receiver.try_recv(), Err(RecvError::Empty));
    }
}

#[test]
fn sender_is_connected() {
    let (sender, receiver) = new_small::<usize>();
    assert!(sender.is_connected());
    drop(receiver);
    assert!(!sender.is_connected());
}

#[test]
fn receiver_is_connected() {
    let (sender, receiver) = new_small::<usize>();
    assert!(receiver.is_connected());
    drop(sender);
    assert!(!receiver.is_connected());
}

#[test]
fn same_channel() {
    let (sender1a, _) = new_small::<usize>();
    let sender1b = sender1a.clone();
    let (sender2a, _) = new_small::<usize>();
    let sender2b = sender2a.clone();

    assert!(sender1a.same_channel(&sender1a));
    assert!(sender1a.same_channel(&sender1b));
    assert!(!sender1a.same_channel(&sender2a));
    assert!(!sender1a.same_channel(&sender2b));
    assert!(sender1b.same_channel(&sender1a));
    assert!(sender1b.same_channel(&sender1b));
    assert!(!sender1b.same_channel(&sender2a));
    assert!(!sender1b.same_channel(&sender2b));

    assert!(!sender2a.same_channel(&sender1a));
    assert!(!sender2a.same_channel(&sender1b));
    assert!(sender2a.same_channel(&sender2a));
    assert!(sender2a.same_channel(&sender2b));
    assert!(!sender2b.same_channel(&sender1a));
    assert!(!sender2b.same_channel(&sender1b));
    assert!(sender2b.same_channel(&sender2a));
    assert!(sender2b.same_channel(&sender2b));
}

#[test]
fn sends_to() {
    let (sender1a, receiver1) = new_small::<usize>();
    let sender1b = sender1a.clone();
    let (sender2a, receiver2) = new_small::<usize>();
    let sender2b = sender2a.clone();

    assert!(sender1a.sends_to(&receiver1));
    assert!(!sender1a.sends_to(&receiver2));
    assert!(sender1b.sends_to(&receiver1));
    assert!(!sender1b.sends_to(&receiver2));

    assert!(!sender2a.sends_to(&receiver1));
    assert!(sender2a.sends_to(&receiver2));
    assert!(!sender2b.sends_to(&receiver1));
    assert!(sender2b.sends_to(&receiver2));
}

mod manager {
    use inbox::{Manager, ReceiverConnected};

    #[test]
    fn new_sender() {
        let (manager, mut sender1, mut receiver) = Manager::<usize>::new_small_channel();
        let mut sender2 = manager.new_sender();

        sender1.try_send(123).unwrap();
        sender2.try_send(456).unwrap();

        assert_eq!(receiver.try_recv().unwrap(), 123);
        assert_eq!(receiver.try_recv().unwrap(), 456);
    }

    #[test]
    fn new_receiver() {
        let (manager, mut sender, receiver) = Manager::<usize>::new_small_channel();
        sender.try_send(123).unwrap();

        drop(receiver);
        sender.try_send(456).unwrap();

        let mut receiver = manager.new_receiver().unwrap();

        assert_eq!(receiver.try_recv().unwrap(), 123);
        assert_eq!(receiver.try_recv().unwrap(), 456);
    }

    #[test]
    fn new_receiver_already_exists() {
        let (manager, _sender, _receiver) = Manager::<usize>::new_small_channel();
        assert_eq!(manager.new_receiver().unwrap_err(), ReceiverConnected);
    }

    #[test]
    fn sending_and_receiving_value() {
        let (manager, mut sender, mut receiver) = Manager::<usize>::new_small_channel();
        sender.try_send(123).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), 123);
        drop(manager);
    }

    #[test]
    fn sender_is_connected() {
        let (manager, sender, receiver) = Manager::<usize>::new_small_channel();
        assert!(sender.is_connected());
        drop(receiver);
        // Manager is still alive.
        assert!(sender.is_connected());
        drop(manager);
        assert!(!sender.is_connected());
    }

    #[test]
    fn receiver_is_connected() {
        let (manager, sender, receiver) = Manager::<usize>::new_small_channel();
        assert!(receiver.is_connected());
        drop(manager);
        assert!(receiver.is_connected());
        drop(sender);
        assert!(!receiver.is_connected());

        let (manager, sender, receiver) = Manager::<usize>::new_small_channel();
        assert!(receiver.is_connected());
        drop(sender);
        assert!(!receiver.is_connected());
        let new_sender = manager.new_sender();
        assert!(receiver.is_connected());
        drop(new_sender);
        assert!(!receiver.is_connected());
    }

    #[test]
    fn same_channel() {
        let (manager1, sender1a, _) = Manager::<usize>::new_small_channel();
        let sender1b = manager1.new_sender();
        let (manager2, sender2a, _) = Manager::<usize>::new_small_channel();
        let sender2b = manager2.new_sender();

        assert!(sender1a.same_channel(&sender1a));
        assert!(sender1a.same_channel(&sender1b));
        assert!(!sender1a.same_channel(&sender2a));
        assert!(!sender1a.same_channel(&sender2b));
        assert!(sender1b.same_channel(&sender1a));
        assert!(sender1b.same_channel(&sender1b));
        assert!(!sender1b.same_channel(&sender2a));
        assert!(!sender1b.same_channel(&sender2b));

        assert!(!sender2a.same_channel(&sender1a));
        assert!(!sender2a.same_channel(&sender1b));
        assert!(sender2a.same_channel(&sender2a));
        assert!(sender2a.same_channel(&sender2b));
        assert!(!sender2b.same_channel(&sender1a));
        assert!(!sender2b.same_channel(&sender1b));
        assert!(sender2b.same_channel(&sender2a));
        assert!(sender2b.same_channel(&sender2b));
    }

    #[test]
    fn sends_to() {
        let (manager1, sender1a, receiver1) = Manager::<usize>::new_small_channel();
        let sender1b = manager1.new_sender();
        let (manager2, sender2a, receiver2) = Manager::<usize>::new_small_channel();
        let sender2b = manager2.new_sender();

        assert!(sender1a.sends_to(&receiver1));
        assert!(!sender1a.sends_to(&receiver2));
        assert!(sender1b.sends_to(&receiver1));
        assert!(!sender1b.sends_to(&receiver2));

        assert!(!sender2a.sends_to(&receiver1));
        assert!(sender2a.sends_to(&receiver2));
        assert!(!sender2b.sends_to(&receiver1));
        assert!(sender2b.sends_to(&receiver2));
    }
}
