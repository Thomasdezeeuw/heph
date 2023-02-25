//! Tests using multiple threads.

use std::thread;
use std::time::Duration;

use heph_inbox::{self as inbox, new, Manager, RecvError, SendError};

#[macro_use]
mod util;

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn send_single_value() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);

        start_threads!(
            {
                expect_send!(sender, 1);
            },
            {
                expect_recv!(receiver, 1);
            }
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn send_single_value_peek() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);

        start_threads!(
            {
                expect_send!(sender, 1);
            },
            {
                expect_peek!(receiver, 1);
            }
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn zero_sized_types() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new(capacity);

        start_threads!(
            {
                expect_send!(sender, ());
            },
            {
                expect_recv!(receiver, ());
            }
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn zero_sized_types_peek() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new(capacity);

        start_threads!(
            {
                expect_send!(sender, ());
            },
            {
                expect_peek!(receiver, ());
            }
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn receive_no_sender() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);

        start_threads!(
            {
                drop(sender);
            },
            {
                r#loop! {
                    match receiver.try_recv() {
                        Ok(..) => panic!("unexpected receive of value"),
                        Err(RecvError::Empty) => {} // Try again.
                        Err(RecvError::Disconnected) => break,
                    }
                }
            }
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn peek_no_sender() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);

        start_threads!(
            {
                drop(sender);
            },
            {
                r#loop! {
                    match receiver.try_peek() {
                        Ok(..) => panic!("unexpected peek of value"),
                        Err(RecvError::Empty) => {} // Try again.
                        Err(RecvError::Disconnected) => break,
                    }
                }
            }
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't support `sleep`.
fn send_no_receiver() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new::<usize>(capacity);

        start_threads!(
            {
                thread::sleep(Duration::from_millis(1));
                r#loop! {
                    match sender.try_send(1) {
                        Ok(()) => {} // Try again.
                        Err(SendError::Full(..)) => panic!("too slow!"),
                        Err(SendError::Disconnected(..)) => break,
                    }
                }
            },
            {
                drop(receiver);
            }
        );
    });
}

#[test]
fn sender_is_connected() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new::<usize>(capacity);

        start_threads!(
            {
                r#loop! {
                    if !sender.is_connected() {
                        break;
                    }
                }
            },
            {
                drop(receiver);
            }
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn receiver_is_connected() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new::<usize>(capacity);

        start_threads!(
            {
                drop(sender);
            },
            {
                r#loop! {
                    if !receiver.is_connected() {
                        break;
                    }
                }
            }
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn sender_has_manager() {
    let (manager, sender, _) = Manager::<()>::new_small_channel();

    start_threads!(
        {
            drop(manager);
        },
        {
            r#loop! {
                if !sender.has_manager() {
                    break;
                }
            }
        }
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Doesn't finish.
fn receiver_has_manager() {
    let (manager, _, receiver) = Manager::<()>::new_small_channel();

    start_threads!(
        {
            drop(manager);
        },
        {
            r#loop! {
                if !receiver.has_manager() {
                    break;
                }
            }
        }
    );
}
