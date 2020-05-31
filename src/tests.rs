// TODO:
// * add Receiver::recv Future: store waker in AtomicU128? 0 being invalid?
// * add Stream impl for Receiver?

// TODO: test:
// * Sender::try_send.
// * Sender::send (SendValue Future).
// * Receiver::try_recv.
// * Receiver::recv (Future).
// * Receiver::wake_next_sender.
// * All of LinkedList.
// * All of Manager.
//   * {Sender, Receiver}::has_manager.
// * Additional drop tests.

mod internal {
    //! Tests internal API.

    use std::mem::size_of;

    use crate::{
        has_status, receiver_pos, slot_status, ALL_STATUSES_MASK, EMPTY, FILLED, LEN, MARK_EMPTIED,
        MARK_NEXT_POS, MARK_READING, POS_BITS, READING, TAKEN,
    };

    #[test]
    fn assertions() {
        // Various assertions that must be true for the channel to work
        // correctly.

        // Enough bits for the statuses of the slots.
        assert_eq!(2_usize.pow(POS_BITS as u32), LEN);

        // Status are different.
        assert_ne!(EMPTY, TAKEN);
        assert_ne!(EMPTY, FILLED);
        assert_ne!(EMPTY, READING);
        assert_ne!(TAKEN, FILLED);
        assert_ne!(TAKEN, READING);
        assert_ne!(FILLED, READING);

        // Slot status marking operations.
        // In `Sender::try_send`.
        assert_eq!(EMPTY | TAKEN, TAKEN);
        assert_eq!(TAKEN | TAKEN, TAKEN);
        assert_eq!(EMPTY | FILLED, FILLED);
        assert_eq!(TAKEN | FILLED, FILLED);
        // In `Receiver::try_recv`.
        assert_eq!(FILLED ^ MARK_READING, READING);
        assert_eq!(FILLED & !MARK_EMPTIED, EMPTY);
        assert_eq!(READING & !MARK_EMPTIED, EMPTY);

        // Changing `Receiver` position doesn't change status of slots.
        const ORIGINAL_STATUS: usize = 0b1110010011100100;
        assert_eq!(
            (size_of::<usize>() * 8) - (ORIGINAL_STATUS.leading_zeros() as usize),
            2 * LEN
        );
        let mut status: usize = ORIGINAL_STATUS.wrapping_sub(MARK_NEXT_POS);
        status = status.wrapping_add(MARK_NEXT_POS);
        assert_eq!(status, ORIGINAL_STATUS);
        status = status.wrapping_add(MARK_NEXT_POS);
        assert_eq!(status & ALL_STATUSES_MASK, ORIGINAL_STATUS);
    }

    #[test]
    fn test_slot_status() {
        let tests = &[
            (0b00, 0, EMPTY),
            (0b01, 0, TAKEN),
            (0b10, 0, READING),
            (0b11, 0, FILLED),
            // Slot 1.
            (0b0000, 1, EMPTY),
            (0b0100, 1, TAKEN),
            (0b1000, 1, READING),
            (0b1100, 1, FILLED),
            // Slot 2.
            (0b000000, 2, EMPTY),
            (0b010000, 2, TAKEN),
            (0b100000, 2, READING),
            (0b110000, 2, FILLED),
            // Slot 3.
            (0b00000000, 3, EMPTY),
            (0b01000000, 3, TAKEN),
            (0b10000000, 3, READING),
            (0b11000000, 3, FILLED),
        ];

        for (input, slot, want) in tests.into_iter().copied() {
            assert_eq!(
                slot_status(input, slot),
                want,
                "input: {:064b}, slot: {}",
                input,
                slot,
            );
        }
    }

    #[test]
    fn test_has_status() {
        let tests = &[
            // Slot 0.
            (0b00, 0, EMPTY, true),
            (0b00, 0, TAKEN, false),
            (0b00, 0, FILLED, false),
            (0b01, 0, EMPTY, false),
            (0b01, 0, TAKEN, true),
            (0b01, 0, FILLED, false),
            (0b11, 0, EMPTY, false),
            (0b11, 0, TAKEN, false),
            (0b11, 0, FILLED, true),
            // Slot 1.
            (0b0000, 1, EMPTY, true),
            (0b0000, 1, TAKEN, false),
            (0b0000, 1, FILLED, false),
            (0b0100, 1, EMPTY, false),
            (0b0100, 1, TAKEN, true),
            (0b0100, 1, FILLED, false),
            (0b1100, 1, EMPTY, false),
            (0b1100, 1, TAKEN, false),
            (0b1100, 1, FILLED, true),
            // Slot 1 filled, check slot 0.
            (0b0100, 0, EMPTY, true),
            (0b0100, 0, TAKEN, false),
            (0b0100, 0, FILLED, false),
            (0b1100, 0, EMPTY, true),
            (0b1100, 0, TAKEN, false),
            (0b1100, 0, FILLED, false),
        ];

        for (input, slot, expected, want) in tests.into_iter().copied() {
            assert_eq!(
                has_status(input, slot, expected),
                want,
                "input: {:064b}, slot: {}, expected: {:02b}",
                input,
                slot,
                expected
            );
        }
    }

    #[test]
    fn test_receiver_pos() {
        let tests = &[
            (0b000_1110010011100100, 0),
            (0b001_1110010011100100, 1),
            (0b010_1110010011100100, 2),
            (0b011_1110010011100100, 3),
            (0b100_1110010011100100, 4),
            (0b101_1110010011100100, 5),
            (0b110_1110010011100100, 6),
            (0b111_1110010011100100, 7),
            // Additional bits are ignored.
            (0b1_000_1110010011100100, 0),
            (0b1_001_1110010011100100, 1),
            (0b1_010_1110010011100100, 2),
            (0b1_011_1110010011100100, 3),
            (0b1_100_1110010011100100, 4),
            (0b1_101_1110010011100100, 5),
            (0b1_110_1110010011100100, 6),
            (0b1_111_1110010011100100, 7),
        ];

        for (input, want) in tests.into_iter().copied() {
            assert_eq!(receiver_pos(input), want, "input: {:064b}", input);
        }
    }
}

mod functional {
    //! Tests functional API.

    use crate::{new_small, RecvError, SendError, LEN};

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
        for value in 0..LEN {
            sender.try_send(value).unwrap();
        }
        assert_eq!(sender.try_send(LEN + 1), Err(SendError::Full(LEN + 1)));
        drop(receiver);
    }

    #[test]
    fn send_len_values_send_then_recv() {
        let (mut sender, mut receiver) = new_small::<usize>();
        for value in 0..LEN {
            sender.try_send(value).unwrap();
        }
        assert!(sender.try_send(LEN + 1).is_err());
        for value in 0..LEN {
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Empty);
    }

    #[test]
    fn send_len_values_interleaved() {
        let (mut sender, mut receiver) = new_small::<usize>();
        for value in 0..LEN {
            sender.try_send(value).unwrap();
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
    }

    #[test]
    fn send_2_len_values_send_then_recv() {
        let (mut sender, mut receiver) = new_small::<usize>();
        for value in 0..LEN {
            sender.try_send(value).unwrap();
        }
        for value in 0..LEN {
            assert_eq!(receiver.try_recv().unwrap(), value);
            sender.try_send(LEN + value).unwrap();
        }
        for value in 0..LEN {
            assert_eq!(receiver.try_recv().unwrap(), LEN + value);
        }
    }

    #[test]
    fn send_2_len_values_interleaved() {
        let (mut sender, mut receiver) = new_small::<usize>();
        for value in 0..2 * LEN {
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
        for value in 0..LEN {
            sender.try_send(value).unwrap();
        }
        drop(sender);
        for value in 0..LEN {
            assert_eq!(receiver.try_recv().unwrap(), value);
        }
        assert_eq!(receiver.try_recv().unwrap_err(), RecvError::Disconnected);
    }

    #[test]
    fn sender_disconnected_after_send_2_len() {
        let (mut sender, mut receiver) = new_small::<usize>();
        for value in 0..2 * LEN {
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
        for n in 1..=(LEN - 1) {
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
}

mod drop_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    use crate::new_small;

    /// Message type used in drop tests to ensure we don't drop undefined memory.
    #[derive(Debug)]
    pub struct NeverDrop;

    impl Drop for NeverDrop {
        fn drop(&mut self) {
            panic!("Dropped uninitialised memory!");
        }
    }

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

    /// Message type used in drop tests.
    #[derive(Debug)]
    pub struct DropTest(Arc<AtomicUsize>);

    impl DropTest {
        /// Create a new `DropTest` message.
        pub fn new() -> (DropTest, IsDropped) {
            let shared = Arc::new(AtomicUsize::new(0));
            (DropTest(shared.clone()), IsDropped(shared))
        }
    }

    impl Drop for DropTest {
        fn drop(&mut self) {
            let _ = self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Type that checks if `DropTest` is actually dropped once and only once.
    #[derive(Debug)]
    pub struct IsDropped(Arc<AtomicUsize>);

    impl Drop for IsDropped {
        fn drop(&mut self) {
            let dropped = self.0.load(Ordering::Acquire);
            if dropped != 1 {
                if thread::panicking() {
                    eprintln!("Dropped item {} times, but already panicking", dropped);
                } else {
                    panic!("Dropped item {} times", dropped);
                }
            }
        }
    }

    #[test]
    fn send_single_value() {
        let (mut sender, receiver) = new_small();
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        drop(sender);
        drop(receiver);

        let (mut sender, receiver) = new_small();
        let (value, _check) = DropTest::new();
        sender.try_send(value).unwrap();
        drop(receiver);
        drop(sender);
    }
}
