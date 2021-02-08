//! Module containing the schedulers and related types.

use std::mem::MaybeUninit;

use crate::rt::process::{ProcessData, ProcessId};

mod local;
mod priority;

#[cfg(test)]
mod tests;

pub(super) use local::LocalScheduler;

pub use priority::Priority;

// TODO: make all fields in `AddActor` private.

/// A handle to add a process to the scheduler.
///
/// This allows the `ProcessId` to be determined before the process is actually
/// added. This is used in registering with the system poller.
pub(super) struct AddActor<I, P: ?Sized> {
    pub(crate) processes: I,
    /// Already allocated `ProcessData`, used to determine the `ProcessId`.
    pub(crate) alloc: Box<MaybeUninit<ProcessData<P>>>,
}

impl<I, P: ?Sized> AddActor<I, P> {
    /// Get the would be `ProcessId` for the process.
    pub(super) const fn pid(&self) -> ProcessId {
        #[allow(trivial_casts)]
        ProcessId(unsafe { &*self.alloc as *const _ as *const u8 as usize })
    }
}
