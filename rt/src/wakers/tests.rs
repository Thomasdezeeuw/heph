mod shared {
    use std::pin::Pin;
    use std::sync::OnceLock;
    use std::sync::{Arc, Weak};
    use std::task::{self, Poll};
    use std::thread::{self, sleep};
    use std::time::Duration;

    use crate::scheduler::process::{self, ProcessId};
    use crate::shared::RuntimeInternals;
    use crate::spawn::options::Priority;
    use crate::wakers::shared::Wakers;

    const PID1: ProcessId = ProcessId(1);
    const PID2: ProcessId = ProcessId(2);

    pub(super) struct TestProcess;

    impl process::Run for TestProcess {
        fn name(&self) -> &'static str {
            "TestProcess"
        }

        fn run(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
            unimplemented!();
        }
    }

    #[test]
    fn waker() {
        let shared_internals = new_internals();

        let pid = shared_internals.add_new_process(Priority::NORMAL, TestProcess);
        assert!(shared_internals.has_process());
        assert!(shared_internals.has_ready_process());
        let process = shared_internals.remove_process().unwrap();
        shared_internals.add_back_process(process);
        assert!(shared_internals.has_process());
        assert!(!shared_internals.has_ready_process());

        // Create a new waker.
        let waker = shared_internals.new_task_waker(pid);

        // Waking should move the process to the ready queue.
        waker.wake_by_ref();
        assert!(shared_internals.has_process());
        assert!(shared_internals.has_ready_process());
        let process = shared_internals.remove_process().unwrap();
        assert_eq!(process.id(), pid);

        // Waking a process that isn't in the scheduler should be fine.
        waker.wake();
        assert!(!shared_internals.has_process());
        assert!(!shared_internals.has_ready_process());
        shared_internals.complete(process);
        assert!(!shared_internals.has_process());
        assert!(!shared_internals.has_ready_process());
    }

    #[test]
    fn cloned_waker() {
        let shared_internals = new_internals();

        // Add a test process.
        let pid = shared_internals.add_new_process(Priority::NORMAL, TestProcess);
        assert!(shared_internals.has_process());
        assert!(shared_internals.has_ready_process());
        let process = shared_internals.remove_process().unwrap();
        shared_internals.add_back_process(process);
        assert!(shared_internals.has_process());
        assert!(!shared_internals.has_ready_process());

        // Create a cloned waker.
        let waker1 = shared_internals.new_task_waker(pid);
        let waker2 = waker1.clone();
        drop(waker1);

        // Waking should move the process to the ready queue.
        waker2.wake();
        assert!(shared_internals.has_process());
        assert!(shared_internals.has_ready_process());
        let process = shared_internals.remove_process().unwrap();
        assert_eq!(process.id(), pid);
    }

    #[test]
    fn wake_from_different_thread() {
        let shared_internals = new_internals();

        let pid = shared_internals.add_new_process(Priority::NORMAL, TestProcess);
        assert!(shared_internals.has_process());
        assert!(shared_internals.has_ready_process());
        let process = shared_internals.remove_process().unwrap();
        shared_internals.add_back_process(process);
        assert!(shared_internals.has_process());
        assert!(!shared_internals.has_ready_process());

        let shared_internals2 = shared_internals.clone();
        let handle = thread::spawn(move || {
            let waker = shared_internals2.new_task_waker(pid);
            waker.wake_by_ref();
            waker.wake();
        });

        loop {
            if let Some(process) = shared_internals.remove_process() {
                assert_eq!(process.id(), pid);
                shared_internals.complete(process);
                break;
            }

            sleep(Duration::from_millis(1));
        }

        handle.join().unwrap();
    }

    #[test]
    fn no_internals() {
        let wakers = Wakers::new(Weak::new());
        let waker = wakers.new_task_waker(PID1);

        // This shouldn't be a problem.
        waker.wake_by_ref();
        waker.wake();
    }

    #[test]
    fn will_wake() {
        let wakers = Wakers::new(Weak::new());
        let waker1a = wakers.new_task_waker(PID1);
        let waker1b = wakers.new_task_waker(PID1);
        let waker2a = wakers.new_task_waker(PID2);
        let waker2b = waker2a.clone();

        assert!(waker1a.will_wake(&waker1a));
        assert!(waker1a.will_wake(&waker1b));
        assert!(!waker1a.will_wake(&waker2a));
        assert!(!waker1a.will_wake(&waker2b));

        assert!(waker1b.will_wake(&waker1a));
        assert!(waker1b.will_wake(&waker1b));
        assert!(!waker1b.will_wake(&waker2a));
        assert!(!waker1b.will_wake(&waker2b));

        assert!(!waker2a.will_wake(&waker1a));
        assert!(!waker2a.will_wake(&waker1b));
        assert!(waker2a.will_wake(&waker2a));
        assert!(waker2a.will_wake(&waker2b));
    }

    pub(super) fn new_internals() -> Arc<RuntimeInternals> {
        RuntimeInternals::new(noop_waker(), None).unwrap()
    }

    fn noop_waker() -> a10::SubmissionQueue {
        static NOOP_WAKER: OnceLock<a10::SubmissionQueue> = OnceLock::new();
        NOOP_WAKER
            .get_or_init(|| {
                let ring = a10::Ring::new().expect("failed to create `a10::Ring` for test module");
                ring.sq().clone()
            })
            .clone()
    }
}
