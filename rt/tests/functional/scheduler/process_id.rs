use heph_rt::setup::scheduler::ProcessId;

use crate::util::assert_size;

const PID1: ProcessId = ProcessId::new(0);
const PID2: ProcessId = ProcessId::new(1);
const PID3: ProcessId = ProcessId::new(2);
const PID4: ProcessId = ProcessId::new(3);

#[test]
fn size_assertion() {
    assert_size::<ProcessId>(8);
}

#[test]
#[allow(clippy::eq_op)] // Need to compare pids to themselves.
fn data_equality() {
    assert_eq!(PID1, PID1);
    assert_ne!(PID1, PID2);
    assert_ne!(PID1, PID3);
    assert_ne!(PID1, PID4);

    assert_ne!(PID2, PID1);
    assert_eq!(PID2, PID2);
    assert_ne!(PID2, PID3);
    assert_ne!(PID2, PID4);

    assert_ne!(PID3, PID1);
    assert_ne!(PID3, PID2);
    assert_eq!(PID3, PID3);
    assert_ne!(PID3, PID4);

    assert_ne!(PID4, PID1);
    assert_ne!(PID4, PID2);
    assert_ne!(PID4, PID3);
    assert_eq!(PID4, PID4);
}

#[test]
fn order() {
    assert!(PID1 < PID2);
    assert!(PID1 < PID3);
    assert!(PID1 < PID4);

    assert!(PID2 > PID1);
    assert!(PID2 < PID3);
    assert!(PID2 < PID4);

    assert!(PID3 > PID1);
    assert!(PID3 > PID2);
    assert!(PID3 < PID4);

    assert!(PID4 > PID1);
    assert!(PID4 > PID2);
    assert!(PID4 > PID3);
}

#[test]
fn fmt() {
    assert_eq!(ProcessId::new(0).to_string(), "0");
    assert_eq!(ProcessId::new(100).to_string(), "100");
    assert_eq!(ProcessId::new(8000).to_string(), "8000");
}
