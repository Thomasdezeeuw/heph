//! Module containing the futures `Wake` implementation.

use std::sync::Arc;
use std::task::{self, Wake, LocalWaker};

use process::ProcessId;
use system::ActorSystemRef;

#[derive(Debug)]
pub struct Waker {
    /// The process to wake up.
    pid: ProcessId,
    /// Reference to the system to wake up the process.
    system_ref: ActorSystemRef,
}

impl Waker {
    pub fn new(pid: ProcessId, system_ref: ActorSystemRef) -> LocalWaker {
        let waker = Waker { pid, system_ref };
        unsafe { task::local_waker(Arc::new(waker)) }
    }
}

impl Wake for Waker {
    fn wake(arc_self: &Arc<Self>) {
        let mut system_ref = arc_self.system_ref.clone();
        if let Err(()) = system_ref.schedule(arc_self.pid) {
            error!("can't wake up actor, actor system shutdown");
        }
    }
}

// This is very unsafe.
unsafe impl Sync for Waker { }
unsafe impl Send for Waker { }
