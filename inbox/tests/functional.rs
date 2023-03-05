//! Functional tests.

use heph_inbox::{
    self as inbox, new, Manager, Receiver, RecvError, SendError, SendValue, Sender, MAX_CAP,
};

#[macro_use]
mod util;

use util::{assert_send, assert_sync};

#[test]
fn sender_is_send() {
    assert_send::<Sender<()>>();
}

#[test]
fn sender_is_sync() {
    assert_sync::<Sender<()>>();
}

#[test]
fn receiver_is_send() {
    assert_send::<Receiver<()>>();
}

#[test]
fn receiver_is_sync() {
    assert_sync::<Receiver<()>>();
}

#[test]
fn manager_is_send() {
    assert_send::<Manager<()>>();
}

#[test]
fn manager_is_sync() {
    assert_sync::<Manager<()>>();
}

#[test]
fn send_value_is_send() {
    assert_send::<SendValue<'_, ()>>();
}

#[test]
fn send_value_is_sync() {
    assert_sync::<SendValue<'_, ()>>();
}

#[test]
#[should_panic = "inbox channel capacity must be between 1 and 29"]
fn capacity_of_zero_should_panic() {
    let _ = inbox::new::<()>(0);
}

#[test]
#[should_panic = "inbox channel capacity must be between 1 and 29"]
fn capacity_too_large_should_panic() {
    let _ = inbox::new::<()>(MAX_CAP + 1);
}

#[test]
fn capacities_are_correct() {
    for capacity in 1..=MAX_CAP {
        let (sender, receiver) = inbox::new::<()>(capacity);
        assert_eq!(sender.capacity(), capacity);
        assert_eq!(receiver.capacity(), capacity);
    }
}

#[test]
fn identifiers() {
    with_all_capacities!(|capacity| {
        let (sender1a, receiver1a) = new::<()>(capacity);
        let sender1b = sender1a.clone();
        let (sender2a, receiver2a) = new::<()>(capacity);
        let sender2b = sender2a.clone();

        assert_eq!(sender1a.id(), sender1a.id());
        assert_eq!(sender1a.id(), sender1b.id());
        assert_eq!(sender1a.id(), receiver1a.id());
        assert_eq!(receiver1a.id(), sender1a.id());
        assert_eq!(receiver1a.id(), sender1b.id());
        assert_eq!(receiver1a.id(), receiver1a.id());

        assert_ne!(sender1a.id(), sender2a.id());
        assert_ne!(sender1a.id(), sender2b.id());
        assert_ne!(sender1a.id(), receiver2a.id());
        assert_ne!(receiver1a.id(), sender2a.id());
        assert_ne!(receiver1a.id(), sender2b.id());
        assert_ne!(receiver1a.id(), receiver2a.id());
    });
}

#[test]
fn sending_and_receiving_value() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        sender.try_send(123).unwrap();
        assert_eq!(receiver.try_peek().unwrap(), &123);
        assert_eq!(receiver.try_recv().unwrap(), 123);
    });
}

#[test]
fn receiving_from_empty_channel() {
    with_all_capacities!(|capacity| {
        let (_sender, mut receiver) = new::<usize>(capacity);
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Empty);
    });
}

#[test]
fn receiving_from_disconnected_channel() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        drop(sender);
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
    });
}

#[test]
fn multiple_peeks() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        sender.try_send(123).unwrap();
        assert_eq!(receiver.try_peek().unwrap(), &123);
        assert_eq!(receiver.try_peek().unwrap(), &123);
        assert_eq!(receiver.try_peek().unwrap(), &123);
    });
}

#[test]
fn peeking_from_empty_channel() {
    with_all_capacities!(|capacity| {
        let (_sender, mut receiver) = new::<usize>(capacity);
        assert_eq!(receiver.try_peek().unwrap_err(), RecvError::Empty);
    });
}

#[test]
fn peeking_from_disconnected_channel() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        drop(sender);
        assert_eq!(receiver.try_peek().unwrap_err(), RecvError::Disconnected);
    });
}

#[test]
fn sending_into_full_channel() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new::<usize>(capacity);
        for value in 0..capacity {
            sender.try_send(value).unwrap();
        }
        assert_eq!(
            sender.try_send(capacity + 1),
            Err(SendError::Full(capacity + 1))
        );
        drop(receiver);
    });
}

#[test]
fn send_len_values_send_then_recv() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        for value in 0..capacity {
            sender.try_send(value).unwrap();
        }
        assert!(sender.try_send(capacity + 1).is_err());
        for value in 0..capacity {
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Empty);
    });
}

#[test]
fn send_len_values_interleaved() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        for value in 0..capacity {
            sender.try_send(value).unwrap();
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
    });
}

#[test]
fn send_2_len_values_send_then_recv() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        for value in 0..capacity {
            sender.try_send(value).unwrap();
        }
        for value in 0..capacity {
            assert_eq!(receiver.try_recv().unwrap(), value);
            sender.try_send(capacity + value).unwrap();
        }
        for value in 0..capacity {
            assert_eq!(receiver.try_recv().unwrap(), capacity + value);
        }
    });
}

#[test]
fn send_2_len_values_interleaved() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        for value in 0..2 * capacity {
            sender.try_send(value).unwrap();
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
    });
}

#[test]
fn sender_disconnected_after_send() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        sender.try_send(123).unwrap();
        drop(sender);
        assert_eq!(receiver.try_recv().unwrap(), 123);
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
    });
}

#[test]
fn sender_disconnected_after_send_len() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        for value in 0..capacity {
            sender.try_send(value).unwrap();
        }
        drop(sender);
        for value in 0..capacity {
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
    });
}

#[test]
fn sender_disconnected_after_send_2_len() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        for value in 0..2 * capacity {
            sender.try_send(value).unwrap();
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
        drop(sender);
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
    });
}

const LARGE: usize = 1_000_000;

#[test]
#[cfg_attr(miri, ignore)] // Takes too long.
fn stress_sending_interleaved() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);
        for value in 0..LARGE {
            sender.try_send(value).unwrap();
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
        assert_eq!(receiver.try_recv(), Err(RecvError::Empty));
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Takes too long.
fn stress_sending_fill() {
    // NOTE: this test is only run for a limited number of capacities, because
    // it doesn't work for all of them. The problem is the wrap around of the
    // receiver position with capacities that are **not** a power of two.
    //
    // For example taking a capacity of 5. The maximum receiver position is 63
    // (6 bits), which is index 3. The next value will be 0 (with wraps around),
    // which is index 0. This means we've skipped index 4.
    //
    // The crate doesn't guaranteed FIFO ordering so it's fine, but this test
    // assumes it.

    /// Iterator for all powers of two in the range `1..=MAX_CAP`.
    struct PowersOfTwo(usize);

    impl Iterator for PowersOfTwo {
        type Item = usize;

        fn next(&mut self) -> Option<Self::Item> {
            if self.0 > MAX_CAP {
                None
            } else {
                let value = self.0;
                self.0 <<= 1;
                Some(value)
            }
        }
    }
    for capacity in PowersOfTwo(1) {
        let (sender, mut receiver) = new::<usize>(capacity);

        let mut value = 0;
        for _ in 0..(LARGE / capacity) {
            for n in 0..capacity {
                sender.try_send(value + n).unwrap();
            }
            for n in 0..capacity {
                assert_eq!(receiver.try_recv().unwrap(), value + n);
            }
            value += capacity;
        }

        assert_eq!(receiver.try_recv(), Err(RecvError::Empty));
    }
}

#[test]
fn sender_is_connected() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new::<usize>(capacity);
        assert!(sender.is_connected());
        drop(receiver);
        assert!(!sender.is_connected());
    });
}

#[test]
fn receiver_is_connected() {
    with_all_capacities!(|capacity| {
        let (sender, receiver) = new::<usize>(capacity);
        assert!(receiver.is_connected());
        drop(sender);
        assert!(!receiver.is_connected());
    });
}

#[test]
fn same_channel() {
    with_all_capacities!(|capacity| {
        let (sender1a, _) = new::<usize>(capacity);
        let sender1b = sender1a.clone();
        let (sender2a, _) = new::<usize>(capacity);
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
    });
}

#[test]
fn sends_to() {
    with_all_capacities!(|capacity| {
        let (sender1a, receiver1) = new::<usize>(capacity);
        let sender1b = sender1a.clone();
        let (sender2a, receiver2) = new::<usize>(capacity);
        let sender2b = sender2a.clone();

        assert!(sender1a.sends_to(&receiver1));
        assert!(!sender1a.sends_to(&receiver2));
        assert!(sender1b.sends_to(&receiver1));
        assert!(!sender1b.sends_to(&receiver2));

        assert!(!sender2a.sends_to(&receiver1));
        assert!(sender2a.sends_to(&receiver2));
        assert!(!sender2b.sends_to(&receiver1));
        assert!(sender2b.sends_to(&receiver2));
    });
}

#[test]
fn receiver_new_sender() {
    with_all_capacities!(|capacity| {
        let (sender, mut receiver) = new::<usize>(capacity);

        let sender2 = receiver.new_sender();
        assert!(sender2.sends_to(&receiver));
        assert!(sender2.same_channel(&sender));

        drop(sender);
        assert!(sender2.is_connected());
        assert!(receiver.is_connected());

        sender2.try_send(123).unwrap();
        assert_eq!(receiver.try_recv().unwrap(), 123);

        drop(receiver);
        assert!(!sender2.is_connected());
    });
}

mod future {
    //! Tests for the `Future` implementations.

    use std::cmp::min;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{self, Poll};

    use heph_inbox::{self as inbox, new};

    use crate::util::new_count_waker;

    macro_rules! pin_stack {
        ($fut: ident) => {
            let mut $fut = $fut;
            #[allow(unused_mut)]
            let mut $fut = unsafe { Pin::new_unchecked(&mut $fut) };
        };
    }

    #[test]
    fn send_value() {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new::<usize>(capacity);

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);

            let future = sender.send(10);
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Ok(())));
            assert_eq!(count, 0);
            assert_eq!(receiver.try_recv(), Ok(10));
        });
    }

    #[test]
    fn send_value_full_channel() {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new::<usize>(capacity);
            // Fill the channel.
            for value in 0..capacity {
                sender.try_send(value).unwrap();
            }

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);

            let future = sender.send(capacity);
            pin_stack!(future);

            // Channel should be full.
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
            assert_eq!(count, 0);

            // Receiving a value should wake a sender.
            assert_eq!(receiver.try_recv(), Ok(0));
            assert_eq!(count, 1);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Ok(())));

            for want in 1..capacity + 1 {
                assert_eq!(receiver.try_recv(), Ok(want));
            }
        });
    }

    #[test]
    fn send_many_values() {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new::<usize>(capacity);

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);

            for value in 0..capacity {
                let future = sender.send(value);
                pin_stack!(future);

                assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Ok(())));
                assert_eq!(count, 0);
            }

            for value in 0..capacity {
                assert_eq!(receiver.try_recv(), Ok(value));
            }
        });
    }

    #[test]
    fn send_many_values_interleaved() {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new::<usize>(capacity);

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);

            for value in 0..capacity {
                let future = sender.send(value);
                pin_stack!(future);

                assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Ok(())));
                assert_eq!(count, 0);

                assert_eq!(receiver.try_recv(), Ok(value));
            }
        });
    }

    // Test where `n` sender try to send into a full channel.
    fn send_many_values_full_channel_test(n: usize) {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new::<usize>(capacity);
            // Fill the channel.
            for value in 0..capacity {
                sender.try_send(value).unwrap();
            }

            // Create a `Sender` for each `SendValue` future.
            let mut senders = (0..n).map(|_| sender.clone()).collect::<Vec<_>>();
            let mut senders = &mut *senders;

            // Create a number of `SendValue` futures.
            let mut futures = Vec::with_capacity(n);
            for index in 0..n {
                let (waker, count) = new_count_waker();
                let mut ctx = task::Context::from_waker(&waker);

                // Work around borrow rules: ensure that we only access a single
                // sender in the vector at a time.
                let (head, tail) = senders.split_first_mut().unwrap();
                senders = tail;
                let mut future = Box::pin(head.send(index));

                assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
                assert_eq!(count, 0);

                futures.push((waker, count, future));
            }

            // The sender should be awoken, however because we don't guarantee
            // FIFO we don't the exact order in which the senders are awoken.
            // `Channel::wake_next_sender` use of `Vec::swap_remove` makes it a
            // bit hard to predict so we just check that at least one sender is
            // awoken for each received message.
            while !futures.is_empty() {
                let n = min(capacity, futures.len());
                for _ in 0..n {
                    receiver.try_recv().unwrap();
                }

                let mut send = 0;
                futures.retain_mut(|(waker, count, future)| {
                    if count.get() != 1 {
                        return true;
                    }

                    let mut ctx = task::Context::from_waker(&waker);
                    assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Ok(())));
                    assert_eq!(*count, 1);
                    send += 1;
                    false
                });
                assert!(
                    send == n,
                    "didn't send enough, thus didn't wake enough senders"
                );
            }
        });
    }

    #[test]
    fn send_many_values_full_channel_one_sender() {
        send_many_values_full_channel_test(1);
    }

    #[test]
    fn send_many_values_full_channel_two_senders() {
        send_many_values_full_channel_test(2);
    }

    #[test]
    fn send_many_values_full_channel_three_senders() {
        send_many_values_full_channel_test(3);
    }

    #[test]
    fn send_many_values_full_channel_four_senders() {
        send_many_values_full_channel_test(4);
    }

    #[test]
    fn send_many_values_full_channel_ten_senders() {
        send_many_values_full_channel_test(10);
    }

    #[test]
    fn send_value_supports_polling_with_different_wakers() {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new::<usize>(capacity);

            for _ in 0..capacity {
                sender.try_send(123).unwrap();
            }

            let (waker1, count1) = new_count_waker();
            let (waker2, count2) = new_count_waker();
            let mut ctx1 = task::Context::from_waker(&waker1);
            let mut ctx2 = task::Context::from_waker(&waker2);

            let mut future = Box::pin(sender.send(10));
            assert_eq!(future.as_mut().poll(&mut ctx1), Poll::Pending);
            assert_eq!(future.as_mut().poll(&mut ctx2), Poll::Pending);

            for _ in 0..capacity {
                assert_eq!(receiver.try_recv().unwrap(), 123);
            }
            drop(receiver);

            assert_eq!(count1, 0);
            assert_eq!(count2, 1);
        });
    }

    #[test]
    fn recv_value() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.recv();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            sender.try_send(10).unwrap();
            assert_eq!(count, 1);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(10)));
        });
    }

    #[test]
    fn recv_value_wake_up_optimised() {
        with_all_capacities!(|capacity| {
            if capacity < 2 {
                // Requires us to send a minimum of two values.
                continue;
            }

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);
            let (sender, mut receiver) = new::<usize>(capacity);

            let future = receiver.recv();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            sender.try_send(10).unwrap();
            assert_eq!(count, 1);
            sender.try_send(20).unwrap();
            assert_eq!(count, 1); // Second wake-up optimised away.

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(10)));
        });
    }

    #[test]
    fn recv_value_twice() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);
            let (sender, mut receiver) = new::<usize>(capacity);

            // Create Future and register waker (by polling).
            let future = receiver.recv();
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Send value.
            sender.try_send(10).unwrap();
            assert_eq!(count, 1);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(10)));

            // Create second Future with and use the same waker.
            let future = receiver.recv();
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Send second value.
            sender.try_send(20).unwrap();
            assert_eq!(count, 2);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(20)));
        });
    }

    #[test]
    fn recv_value_twice_senders_dropped() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);
            let (sender, mut receiver) = new::<usize>(capacity);

            // Create Future and register waker (by polling).
            let future = receiver.recv();
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Send value.
            sender.try_send(10).unwrap();
            assert_eq!(count, 1);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(10)));

            // Create second Future with and use the same waker.
            let future = receiver.recv();
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Dropping the second should wake up the receiver.
            drop(sender);
            assert_eq!(count, 2);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(None));
        });
    }

    #[test]
    fn recv_value_empty() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.recv();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
            assert_eq!(count, 0);

            sender.try_send(10).unwrap();

            assert_eq!(count, 1);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(10)));
        });
    }

    #[test]
    fn recv_value_all_senders_disconnected() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.recv();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Dropping the last sender should notify the receiver.
            drop(sender);
            assert_eq!(count, 1);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(None));
        });
    }

    #[test]
    fn recv_value_all_senders_disconnected_not_empty() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.recv();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Sending and dropping the last sender should wake the receiver.
            sender.try_send(10).unwrap();
            assert_eq!(count, 1);
            drop(sender);
            assert_eq!(count, 1); // Wake-up optimised away.

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(10)));
            let mut future = receiver.recv();
            assert_eq!(Pin::new(&mut future).poll(&mut ctx), Poll::Ready(None));
        });
    }

    #[test]
    fn recv_value_all_senders_disconnected_cloned_sender() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);
            let sender2 = sender.clone();

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.recv();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Only dropping the last sender should wake the receiver.
            drop(sender);
            assert_eq!(count, 0);
            drop(sender2);
            assert_eq!(count, 1);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(None));
        });
    }

    #[test]
    fn recv_value_only_wake_if_polled() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.recv();
            pin_stack!(future);

            drop(sender);
            // `RecvValue` isn't polled yet, so we shouldn't receive a wake-up
            // notification.
            assert_eq!(count, 0);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(None));
        });
    }

    #[test]
    fn registered_receiver_waker() {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new::<usize>(capacity);

            let (waker, count) = new_count_waker();
            receiver.register_waker(&waker);

            assert_eq!(count, 0);
            assert_eq!(sender.try_send(10), Ok(()));
            assert_eq!(count, 1);
            assert_eq!(receiver.try_recv(), Ok(10));
        });
    }

    #[test]
    fn forget_send_value() {
        with_all_capacities!(|capacity| {
            let (sender, mut receiver) = new::<usize>(capacity);

            // Fill the channel.
            for n in 0..capacity {
                sender.try_send(n).unwrap();
            }

            // Create the `SendValue` future and poll it once to register the waker.
            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);
            let future = sender.send(10);
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
            std::mem::forget(future);
            assert_eq!(receiver.try_recv(), Ok(0));
            assert_eq!(count, 1);
        });
    }

    #[test]
    fn peek_value() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.peek();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            sender.try_send(10).unwrap();
            assert_eq!(count, 1);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(&10)));
        });
    }

    #[test]
    fn peek_value_twice() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);
            let (sender, mut receiver) = new::<usize>(capacity);

            // Create Future and register waker (by polling).
            let future = receiver.peek();
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Send value.
            sender.try_send(10).unwrap();
            assert_eq!(count, 1);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(&10)));

            // Create second Future with and use the same waker.
            let future = receiver.peek();
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(&10)));
        });
    }

    #[test]
    fn send_two_values_peek_value() {
        with_all_capacities!(|capacity| {
            if capacity < 2 {
                continue;
            }

            let (waker, _count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);
            let (sender, mut receiver) = new::<usize>(capacity);

            // Send value.
            sender.try_send(10).unwrap();
            // Send second value.
            sender.try_send(20).unwrap();

            let future = receiver.peek();
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(&10)));
        });
    }

    #[test]
    fn peek_value_empty() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.peek();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
            assert_eq!(count, 0);

            sender.try_send(10).unwrap();

            assert_eq!(count, 1);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(&10)));
        });
    }

    #[test]
    fn peek_value_all_senders_disconnected() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.peek();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Dropping the last sender should notify the receiver.
            drop(sender);
            assert_eq!(count, 1);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(None));
        });
    }

    #[test]
    fn peek_value_all_senders_disconnected_not_empty() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.peek();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Sending and dropping the last sender should wake the receiver.
            sender.try_send(10).unwrap();
            assert_eq!(count, 1);
            drop(sender);
            assert_eq!(count, 1); // Wake-up optimised away.

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(Some(&10)));
        });
    }

    #[test]
    fn peek_value_all_senders_disconnected_cloned_sender() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);
            let sender2 = sender.clone();

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.peek();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);

            // Only dropping the last sender should wake the receiver.
            drop(sender);
            assert_eq!(count, 0);
            drop(sender2);
            assert_eq!(count, 1);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(None));
        });
    }

    #[test]
    fn peek_value_only_wake_if_polled() {
        with_all_capacities!(|capacity| {
            let (waker, count) = new_count_waker();
            let (sender, mut receiver) = new::<usize>(capacity);

            let mut ctx = task::Context::from_waker(&waker);

            let future = receiver.peek();
            pin_stack!(future);

            drop(sender);
            // `PeekValue` isn't polled yet, so we shouldn't receive a wake-up
            // notification.
            assert_eq!(count, 0);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(None));
        });
    }

    #[test]
    fn sender_join() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new::<usize>(capacity);

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);

            let future = sender.join();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
            assert_eq!(count, 0);

            drop(receiver);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(()));
            assert_eq!(count, 1);
        });
    }

    #[test]
    fn sender_join_drop_before_waker_register() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new::<usize>(capacity);
            drop(receiver);

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);

            let future = sender.join();
            pin_stack!(future);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(()));
            assert_eq!(count, 0);
        });
    }

    #[test]
    fn sender_join_dont_register_waker_twice() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new::<usize>(capacity);

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);

            let future = sender.join();
            pin_stack!(future);

            // Poll twice.
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
            assert_eq!(count, 0);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
            assert_eq!(count, 0);

            drop(receiver);
            assert_eq!(future.as_mut().poll(&mut ctx), Poll::Ready(()));
            assert_eq!(count, 1);
        });
    }

    #[test]
    fn sender_join_poll_with_different_wakers() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new::<usize>(capacity);

            let (waker, count1) = new_count_waker();
            let mut ctx1 = task::Context::from_waker(&waker);
            let (waker, count2) = new_count_waker();
            let mut ctx2 = task::Context::from_waker(&waker);

            let future = sender.join();
            pin_stack!(future);

            assert_eq!(future.as_mut().poll(&mut ctx1), Poll::Pending);
            assert_eq!(count1, 0);
            // Poll with a different waker.
            assert_eq!(future.as_mut().poll(&mut ctx2), Poll::Pending);
            assert_eq!(count2, 0);

            drop(receiver);
            assert_eq!(future.as_mut().poll(&mut ctx2), Poll::Ready(()));
            assert_eq!(count1, 0);
            assert_eq!(count2, 1);
        });
    }

    #[test]
    fn sender_join_no_wakeup_after_drop() {
        with_all_capacities!(|capacity| {
            let (sender, receiver) = new::<usize>(capacity);

            let (waker, count) = new_count_waker();
            let mut ctx = task::Context::from_waker(&waker);

            {
                // NOTE: putting `future` in a code block to ensure it's dropped.
                let future = Box::pin(sender.join());
                pin_stack!(future);

                assert_eq!(future.as_mut().poll(&mut ctx), Poll::Pending);
                assert_eq!(count, 0);
            }

            drop(receiver);
            assert_eq!(count, 0);
        });
    }
}

mod manager {
    use heph_inbox::{self as inbox, Manager, ReceiverConnected};

    #[test]
    fn new_sender() {
        let (manager, sender1, mut receiver) = Manager::<usize>::new_channel(2);
        let sender2 = manager.new_sender();

        sender1.try_send(123).unwrap();
        sender2.try_send(456).unwrap();

        assert_eq!(receiver.try_peek().unwrap(), &123);
        assert_eq!(receiver.try_recv().unwrap(), 123);
        assert_eq!(receiver.try_peek().unwrap(), &456);
        assert_eq!(receiver.try_recv().unwrap(), 456);
    }

    #[test]
    fn new_receiver() {
        let (manager, sender, receiver) = Manager::<usize>::new_channel(3);
        sender.try_send(123).unwrap();

        drop(receiver);
        sender.try_send(456).unwrap();

        let mut receiver = manager.new_receiver().unwrap();

        assert_eq!(receiver.try_peek().unwrap(), &123);
        assert_eq!(receiver.try_recv().unwrap(), 123);
        assert_eq!(receiver.try_peek().unwrap(), &456);
        assert_eq!(receiver.try_recv().unwrap(), 456);
    }

    #[test]
    fn new_receiver_already_exists() {
        let (manager, _sender, _receiver) = Manager::<usize>::new_channel(1);
        assert_eq!(manager.new_receiver().unwrap_err(), ReceiverConnected);
    }

    #[test]
    fn sending_and_receiving_value() {
        with_all_capacities!(|capacity| {
            let (manager, sender, mut receiver) = Manager::<usize>::new_channel(capacity);
            sender.try_send(123).unwrap();
            assert_eq!(receiver.try_peek().unwrap(), &123);
            assert_eq!(receiver.try_recv().unwrap(), 123);
            drop(manager);
        });
    }

    #[test]
    fn sender_is_connected() {
        let (manager, sender, receiver) = Manager::<usize>::new_channel(4);
        assert!(sender.is_connected());
        drop(receiver);
        // Manager is still alive.
        assert!(sender.is_connected());
        drop(manager);
        assert!(!sender.is_connected());
    }

    #[test]
    fn receiver_is_connected() {
        let (manager, sender, receiver) = Manager::<usize>::new_channel(5);
        assert!(receiver.is_connected());
        drop(manager);
        assert!(receiver.is_connected());
        drop(sender);
        assert!(!receiver.is_connected());

        let (manager, sender, receiver) = Manager::<usize>::new_channel(6);
        assert!(receiver.is_connected());
        drop(sender);
        assert!(!receiver.is_connected());
        let new_sender = manager.new_sender();
        assert!(receiver.is_connected());
        drop(new_sender);
        assert!(!receiver.is_connected());
    }

    #[test]
    fn sender_has_manager() {
        let (manager, sender, receiver) = Manager::<()>::new_small_channel();
        assert!(sender.has_manager());
        drop(receiver);
        assert!(sender.has_manager());
        drop(manager);
        assert!(!sender.has_manager());
    }

    #[test]
    fn receiver_has_manager() {
        let (manager, sender, receiver) = Manager::<()>::new_small_channel();
        assert!(receiver.has_manager());
        drop(sender);
        assert!(receiver.has_manager());
        drop(manager);
        assert!(!receiver.has_manager());
    }

    #[test]
    fn same_channel() {
        let (manager1, sender1a, _) = Manager::<usize>::new_channel(1);
        let sender1b = manager1.new_sender();
        let (manager2, sender2a, _) = Manager::<usize>::new_channel(1);
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
        let (manager1, sender1a, receiver1) = Manager::<usize>::new_channel(1);
        let sender1b = manager1.new_sender();
        let (manager2, sender2a, receiver2) = Manager::<usize>::new_channel(1);
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
