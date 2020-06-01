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

mod drop_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    use crate::{new_small, Manager, LEN};

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
        let _checks: Vec<IsDropped> = (0..LEN)
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
        let _checks: Vec<IsDropped> = (0..LEN)
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
        let _checks: Vec<IsDropped> = (0..LEN)
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
        let _checks: Vec<IsDropped> = (0..LEN)
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
        let _checks: Vec<IsDropped> = (0..LEN)
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
        let _checks: Vec<IsDropped> = (0..LEN)
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
}
