mod local {
    use std::thread;

    use crate::wakers::Wakers;
    use crate::ProcessId;

    const PID1: ProcessId = ProcessId(0);
    const PID2: ProcessId = ProcessId(usize::MAX);

    #[test]
    fn waker() {
        let mut ring = a10::Ring::new(2).unwrap();

        // Create the wakers.
        let (wake_sender, wake_receiver) = crossbeam_channel::unbounded();
        let mut wakers = Wakers::new(wake_sender, ring.submission_queue().clone());

        // Create a new waker.
        let waker = wakers.new_task_waker(PID1);

        // The waker should wake up the polling.
        waker.wake();
        ring.poll(None).unwrap();
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));

        let waker = wakers.new_task_waker(PID2);
        waker.wake();
        ring.poll(None).unwrap();
        // Should receive an event for the Waker.
        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(PID2));
    }

    #[test]
    fn waker_different_thread() {
        let mut ring = a10::Ring::new(2).unwrap();

        // Create the wakers.
        let (wake_sender, wake_receiver) = crossbeam_channel::unbounded();
        let mut wakers = Wakers::new(wake_sender, ring.submission_queue().clone());

        // Create a new waker.
        let waker = wakers.new_task_waker(PID1);

        // A call to the waker should wake a polling thread.
        let handle = thread::spawn(move || {
            ring.poll(None).unwrap();
        });
        waker.wake();
        handle.join().unwrap();

        // And the process id that needs to be scheduled.
        assert_eq!(wake_receiver.try_recv(), Ok(PID1));
    }
}

mod shared {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Weak};
    use std::task::{self, Poll};
    use std::thread::{self, sleep};
    use std::time::Duration;

    use crate::process::{FutureProcess, Process, ProcessId};
    use crate::shared::RuntimeInternals;
    use crate::spawn::options::Priority;
    use crate::test;
    use crate::wakers::shared::Wakers;

    const PID1: ProcessId = ProcessId(1);
    const PID2: ProcessId = ProcessId(2);

    struct TestProcess;

    impl Future for TestProcess {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<()> {
            unimplemented!();
        }
    }

    impl Process for TestProcess {
        fn name(&self) -> &'static str {
            "TestProcess"
        }
    }

    #[test]
    fn waker() {
        let shared_internals = new_internals();

        let pid = shared_internals.add_new_process(Priority::NORMAL, FutureProcess(TestProcess));
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
        assert_eq!(process.as_ref().id(), pid);

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
        let pid = shared_internals.add_new_process(Priority::NORMAL, FutureProcess(TestProcess));
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
        assert_eq!(process.as_ref().id(), pid);
    }

    #[test]
    fn wake_from_different_thread() {
        let shared_internals = new_internals();

        let pid = shared_internals.add_new_process(Priority::NORMAL, FutureProcess(TestProcess));
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
                assert_eq!(process.as_ref().id(), pid);
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

    fn new_internals() -> Arc<RuntimeInternals> {
        let setup = RuntimeInternals::test_setup(2).unwrap();
        Arc::new_cyclic(|shared_internals| {
            let wakers = Wakers::new(shared_internals.clone());
            let worker_wakers = vec![test::noop_waker()].into_boxed_slice();
            setup.complete(wakers, worker_wakers, None)
        })
    }
}
