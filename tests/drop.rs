//! Tests for memory deallocation.

use heph_inbox::{self as inbox, new, Manager};

#[macro_use]
mod util;

use util::{DropTest, IsDropped, NeverDrop};

#[test]
fn empty() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new::<NeverDrop>(capacity);
        drop(sender);
        drop(receiver);

        let (sender, receiver) = new::<NeverDrop>(capacity);
        drop(sender);
        drop(receiver);
    });
}

#[test]
fn empty_cloned() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new::<NeverDrop>(capacity);
        let sender2 = sender.clone();
        drop(sender);
        drop(sender2);
        drop(receiver);

        let (sender, receiver) = new::<NeverDrop>(capacity);
        let sender2 = sender.clone();
        drop(sender);
        drop(sender2);
        drop(receiver);
    });
}

#[test]
fn empty_with_manager() {
    with_all_capacities!(|capacity| {
        let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
        drop(sender);
        drop(receiver);
        drop(manager);

        let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
        drop(sender);
        drop(receiver);
        drop(manager);

        let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
        drop(manager);
        drop(sender);
        drop(receiver);

        let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
        drop(manager);
        drop(sender);
        drop(receiver);
    });
}

#[test]
fn empty_cloned_with_manager() {
    with_all_capacities!(|capacity| {
        let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
        let sender2 = sender.clone();
        drop(sender);
        drop(sender2);
        drop(receiver);
        drop(manager);

        let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
        let sender2 = sender.clone();
        drop(sender);
        drop(sender2);
        drop(receiver);
        drop(manager);

        let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
        let sender2 = sender.clone();
        drop(manager);
        drop(sender);
        drop(sender2);
        drop(receiver);

        let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
        let sender2 = sender.clone();
        drop(manager);
        drop(sender);
        drop(sender2);
        drop(receiver);
    });
}

#[test]
fn send_single_value_sr() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new(capacity);
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        drop(sender);
        drop(receiver);
    });
}

#[test]
fn send_single_value_rs() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new(capacity);
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        drop(receiver);
        drop(sender);
    });
}

#[test]
fn send_single_value_with_manager() {
    with_all_capacities!(|capacity| {
        let (manager, sender, receiver) = Manager::new_channel(capacity);
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        drop(sender);
        drop(receiver);
        drop(manager);
    });
}

#[test]
fn full_channel_sr() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new(capacity);
        let _checks: Vec<IsDropped> = (0..capacity)
            .into_iter()
            .map(|_| {
                let (value, check) = DropTest::new();
                sender.try_send(value).unwrap();
                check
            })
            .collect();
        drop(sender);
        drop(receiver);
    });
}

#[test]
fn full_channel_rs() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new(capacity);
        let _checks: Vec<IsDropped> = (0..capacity)
            .into_iter()
            .map(|_| {
                let (value, check) = DropTest::new();
                sender.try_send(value).unwrap();
                check
            })
            .collect();
        drop(receiver);
        drop(sender);
    });
}

#[test]
fn full_channel_with_manager() {
    with_all_capacities!(|capacity| {
        let (manager, sender, receiver) = Manager::new_channel(capacity);
        let _checks: Vec<IsDropped> = (0..capacity)
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
    });
}

#[test]
fn value_received_sr() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new(capacity);
        let _checks: Vec<IsDropped> = (0..capacity)
            .into_iter()
            .map(|_| {
                let (value, check) = DropTest::new();
                sender.try_send(value).unwrap();
                check
            })
            .collect();
        let _value1 = receiver.try_recv().unwrap();
        if capacity > 1 {
            let _value2 = receiver.try_recv().unwrap();
        }
        drop(sender);
        drop(receiver);
    });
}

#[test]
fn value_received_rs() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new(capacity);
        let _checks: Vec<IsDropped> = (0..capacity)
            .into_iter()
            .map(|_| {
                let (value, check) = DropTest::new();
                sender.try_send(value).unwrap();
                check
            })
            .collect();
        let _value1 = receiver.try_recv().unwrap();
        if capacity > 1 {
            let _value2 = receiver.try_recv().unwrap();
        }
        drop(receiver);
        drop(sender);
    });
}

#[test]
fn value_received_with_manager() {
    with_all_capacities!(|capacity| {
        let (manager, sender, mut receiver) = Manager::new_channel(capacity);
        let _checks: Vec<IsDropped> = (0..capacity)
            .into_iter()
            .map(|_| {
                let (value, check) = DropTest::new();
                sender.try_send(value).unwrap();
                check
            })
            .collect();
        let _value1 = receiver.try_recv().unwrap();
        if capacity > 1 {
            let _value2 = receiver.try_recv().unwrap();
        }
        drop(receiver);
        drop(sender);
        drop(manager);
    });
}

mod threaded {
    use std::cmp::min;
    use std::thread;
    use std::time::Duration;

    use heph_inbox::{self as inbox, new, Manager};

    use crate::util::{DropTest, NeverDrop};

    #[test]
    fn empty() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new::<NeverDrop>(capacity);

            start_threads!(
                {
                    drop(sender);
                },
                {
                    drop(receiver);
                }
            );
        })
    }

    #[test]
    fn empty_cloned() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new::<NeverDrop>(capacity);
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
        });
    }

    #[test]
    fn empty_with_manager() {
        with_all_capacities!(|capacity| {
            let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);

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
        });
    }

    #[test]
    fn empty_cloned_with_manager() {
        with_all_capacities!(|capacity| {
            let (manager, sender, receiver) = Manager::<NeverDrop>::new_channel(capacity);
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
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // `sleep` not supported.
    fn send_single_value() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new(capacity);
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
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // `sleep` not supported.
    fn send_single_value_with_manager() {
        with_all_capacities!(|capacity| {
            let (manager, sender, receiver) = Manager::new_channel(capacity);
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
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // `sleep` not supported.
    fn full_channel() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new(capacity);
            let (values, _checks) = DropTest::many(capacity);

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
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // `sleep` not supported.
    fn full_channel_with_manager() {
        with_all_capacities!(|capacity| {
            let (manager, sender, receiver) = Manager::new_channel(capacity);
            let (values, _checks) = DropTest::many(capacity);

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
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // `sleep` not supported.
    fn value_received() {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new(capacity);
            let (values, _checks) = DropTest::many(capacity);

            start_threads!(
                {
                    for value in values {
                        expect_send!(sender, value);
                    }
                    drop(sender);
                },
                {
                    let receive_max = min(2, capacity);
                    for _ in 0..receive_max {
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
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // `sleep` not supported.
    fn value_received_with_manager() {
        with_all_capacities!(|capacity| {
            let (manager, sender, mut receiver) = Manager::new_channel(capacity);
            let (values, _checks) = DropTest::many(capacity);

            start_threads!(
                {
                    for value in values {
                        expect_send!(sender, value);
                    }
                    drop(sender);
                },
                {
                    let receive_max = min(2, capacity);
                    for _ in 0..receive_max {
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
        });
    }
}
