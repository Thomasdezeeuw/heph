//! Tests for memory deallocation.

#![feature(once_cell)]

use inbox::{new_small, Manager};

#[macro_use]
mod util;

use util::{DropTest, IsDropped, NeverDrop, SMALL_CAP};

#[test]
fn empty() {
    let (sender, receiver) = new_small::<NeverDrop>();
    drop(sender);
    drop(receiver);

    let (sender, receiver) = new_small::<NeverDrop>();
    drop(sender);
    drop(receiver);
}

#[test]
fn empty_cloned() {
    let (sender, receiver) = new_small::<NeverDrop>();
    let sender2 = sender.clone();
    drop(sender);
    drop(sender2);
    drop(receiver);

    let (sender, receiver) = new_small::<NeverDrop>();
    let sender2 = sender.clone();
    drop(sender);
    drop(sender2);
    drop(receiver);
}

#[test]
fn empty_with_manager() {
    let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
    drop(sender);
    drop(receiver);
    drop(manager);

    let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
    drop(sender);
    drop(receiver);
    drop(manager);

    let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
    drop(manager);
    drop(sender);
    drop(receiver);

    let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
    drop(manager);
    drop(sender);
    drop(receiver);
}

#[test]
fn empty_cloned_with_manager() {
    let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
    let sender2 = sender.clone();
    drop(sender);
    drop(sender2);
    drop(receiver);
    drop(manager);

    let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
    let sender2 = sender.clone();
    drop(sender);
    drop(sender2);
    drop(receiver);
    drop(manager);

    let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
    let sender2 = sender.clone();
    drop(manager);
    drop(sender);
    drop(sender2);
    drop(receiver);

    let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
    let sender2 = sender.clone();
    drop(manager);
    drop(sender);
    drop(sender2);
    drop(receiver);
}

#[test]
fn send_single_value_sr() {
    let (mut sender, receiver) = new_small();
    let (value, _check) = DropTest::new();
    sender.try_send(value).unwrap();
    drop(sender);
    drop(receiver);
}

#[test]
fn send_single_value_rs() {
    let (mut sender, receiver) = new_small();
    let (value, _check) = DropTest::new();
    sender.try_send(value).unwrap();
    drop(receiver);
    drop(sender);
}

#[test]
fn send_single_value_with_manager() {
    let (manager, mut sender, receiver) = Manager::new_small_channel();
    let (value, _check) = DropTest::new();
    sender.try_send(value).unwrap();
    drop(sender);
    drop(receiver);
    drop(manager);
}

#[test]
fn full_channel_sr() {
    let (mut sender, receiver) = new_small();
    let _checks: Vec<IsDropped> = (0..SMALL_CAP)
        .into_iter()
        .map(|_| {
            let (value, check) = DropTest::new();
            sender.try_send(value).unwrap();
            check
        })
        .collect();
    drop(sender);
    drop(receiver);
}

#[test]
fn full_channel_rs() {
    let (mut sender, receiver) = new_small();
    let _checks: Vec<IsDropped> = (0..SMALL_CAP)
        .into_iter()
        .map(|_| {
            let (value, check) = DropTest::new();
            sender.try_send(value).unwrap();
            check
        })
        .collect();
    drop(receiver);
    drop(sender);
}

#[test]
fn full_channel_with_manager() {
    let (manager, mut sender, receiver) = Manager::new_small_channel();
    let _checks: Vec<IsDropped> = (0..SMALL_CAP)
        .into_iter()
        .map(|_| {
            let (value, check) = DropTest::new();
            sender.try_send(value).unwrap();
            check
        })
        .collect();
    drop(sender);
    drop(receiver);
    drop(manager);
}

#[test]
fn value_received_sr() {
    let (mut sender, mut receiver) = new_small();
    let _checks: Vec<IsDropped> = (0..SMALL_CAP)
        .into_iter()
        .map(|_| {
            let (value, check) = DropTest::new();
            sender.try_send(value).unwrap();
            check
        })
        .collect();
    let _value1 = receiver.try_recv().unwrap();
    let _value2 = receiver.try_recv().unwrap();
    drop(sender);
    drop(receiver);
}

#[test]
fn value_received_rs() {
    let (mut sender, mut receiver) = new_small();
    let _checks: Vec<IsDropped> = (0..SMALL_CAP)
        .into_iter()
        .map(|_| {
            let (value, check) = DropTest::new();
            sender.try_send(value).unwrap();
            check
        })
        .collect();
    let _value1 = receiver.try_recv().unwrap();
    let _value2 = receiver.try_recv().unwrap();
    drop(receiver);
    drop(sender);
}

#[test]
fn value_received_with_manager() {
    let (manager, mut sender, mut receiver) = Manager::new_small_channel();
    let _checks: Vec<IsDropped> = (0..SMALL_CAP)
        .into_iter()
        .map(|_| {
            let (value, check) = DropTest::new();
            sender.try_send(value).unwrap();
            check
        })
        .collect();
    let _value1 = receiver.try_recv().unwrap();
    let _value2 = receiver.try_recv().unwrap();
    drop(receiver);
    drop(sender);
    drop(manager);
}

mod threaded {
    use std::thread;
    use std::time::Duration;

    use inbox::{new_small, Manager};

    use super::{DropTest, NeverDrop, SMALL_CAP};

    #[test]
    fn empty() {
        let (sender, receiver) = new_small::<NeverDrop>();

        start_threads!(
            {
                drop(sender);
            },
            {
                drop(receiver);
            }
        );
    }

    #[test]
    fn empty_cloned() {
        let (sender, receiver) = new_small::<NeverDrop>();
        let sender2 = sender.clone();

        start_threads!(
            {
                drop(sender);
            },
            {
                drop(sender2);
            },
            {
                drop(receiver);
            }
        );
    }

    #[test]
    fn empty_with_manager() {
        let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();

        start_threads!(
            {
                drop(sender);
            },
            {
                drop(receiver);
            },
            {
                drop(manager);
            },
        );
    }

    #[test]
    fn empty_cloned_with_manager() {
        let (manager, sender, receiver) = Manager::<NeverDrop>::new_small_channel();
        let sender2 = sender.clone();

        start_threads!(
            {
                drop(sender);
            },
            {
                drop(sender2);
            },
            {
                drop(receiver);
            },
            {
                drop(manager);
            },
        );
    }

    #[test]
    fn send_single_value() {
        let (mut sender, receiver) = new_small();
        let (value, _check) = DropTest::new();

        start_threads!(
            {
                expect_send!(sender, value);
                drop(sender);
            },
            {
                // Give the sender a chance to send the message.
                thread::sleep(Duration::from_millis(10));
                drop(receiver);
            }
        );
    }

    #[test]
    fn send_single_value_with_manager() {
        let (manager, mut sender, receiver) = Manager::new_small_channel();
        let (value, _check) = DropTest::new();

        start_threads!(
            {
                expect_send!(sender, value);
                drop(sender);
            },
            {
                // Give the sender a chance to send the message.
                thread::sleep(Duration::from_millis(10));
                drop(receiver);
            },
            {
                drop(manager);
            },
        );
    }

    #[test]
    fn full_channel() {
        let (mut sender, receiver) = new_small();
        let (values, _checks) = DropTest::many(SMALL_CAP);

        start_threads!(
            {
                for value in values {
                    expect_send!(sender, value);
                }
                drop(sender);
            },
            {
                // Give the sender a chance to send the messages.
                thread::sleep(Duration::from_millis(200));
                drop(receiver);
            }
        );
    }

    #[test]
    fn full_channel_with_manager() {
        let (manager, mut sender, receiver) = Manager::new_small_channel();
        let (values, _checks) = DropTest::many(SMALL_CAP);

        start_threads!(
            {
                for value in values {
                    expect_send!(sender, value);
                }
                drop(sender);
            },
            {
                // Give the sender a chance to send the messages.
                thread::sleep(Duration::from_millis(200));
                drop(receiver);
            },
            {
                drop(manager);
            },
        );
    }

    #[test]
    fn value_received() {
        let (mut sender, mut receiver) = new_small();
        let (values, _checks) = DropTest::many(SMALL_CAP);

        start_threads!(
            {
                for value in values {
                    expect_send!(sender, value);
                }
                drop(sender);
            },
            {
                for _ in 0..2 {
                    r#loop! {
                        match receiver.try_recv() {
                            Ok(..) => break,
                            Err(inbox::RecvError::Empty) => {} // Try again.
                            Err(err) => panic!("unexpected error receiving: {}", err),
                        }
                    }
                }
                // Give the sender a chance to send the remaining messages.
                thread::sleep(Duration::from_millis(200));
                drop(receiver);
            }
        );
    }

    #[test]
    fn value_received_with_manager() {
        let (manager, mut sender, mut receiver) = Manager::new_small_channel();
        let (values, _checks) = DropTest::many(SMALL_CAP);

        start_threads!(
            {
                for value in values {
                    expect_send!(sender, value);
                }
                drop(sender);
            },
            {
                for _ in 0..2 {
                    r#loop! {
                        match receiver.try_recv() {
                            Ok(..) => break,
                            Err(inbox::RecvError::Empty) => {} // Try again.
                            Err(err) => panic!("unexpected error receiving: {}", err),
                        }
                    }
                }
                // Give the sender a chance to send the remaining messages.
                thread::sleep(Duration::from_millis(200));
                drop(receiver);
            },
            {
                drop(manager);
            }
        );
    }
}
