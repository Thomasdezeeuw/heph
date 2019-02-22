//! Module containing the futures `Wake` implementation.

use std::task::Waker;

use crossbeam_channel::Sender;

use crate::scheduler::ProcessId;

/// Create a new `Waker`.
///
/// The implementation will send `ProcessId` into the `sender` channel.
pub fn new_waker(pid: ProcessId, sender: Sender<ProcessId>) -> Waker {
    unimplemented!();
}
