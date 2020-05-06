use std::mem::{forget, replace};
use std::pin::Pin;
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::rt::scheduler::ProcessData;

// TODO: currently this creates and drops Node on almost every operation. Maybe
// we can keep (some of) the structure in place, changing `Node.process` into an
// AtomicPtr as well? Maybe with a tag in the left/right pointer to indicate
// wether or not the `process` is valid (changing `process` to MaybeUninit).

/// Processes that are ready to run.
///
/// Implemented as a simple binary tree.
pub(super) struct RunQueue {
    root: AtomicPtr<Node>,
}

struct Node {
    process: Pin<Box<ProcessData>>,
    left: AtomicPtr<Node>,
    right: AtomicPtr<Node>,
}

impl RunQueue {
    /// Returns an empty `RunQueue`.
    pub(super) const fn empty() -> RunQueue {
        RunQueue {
            root: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// Returns `true` if the queue contains any process.
    pub(super) fn has_process(&self) -> bool {
        !self.root.load(Ordering::Relaxed).is_null()
    }

    /// Add `process` to the queue of running processes.
    pub(super) fn add(&self, process: Pin<Box<ProcessData>>) {
        let new_node = Node::new(process);
        let new_node_ptr = Box::into_raw(new_node);
        // Convenience reference so we don't have to dereference the raw pointer
        // when compare to the node in the loop.
        let new_node = unsafe { &*new_node_ptr };

        let mut next_node = &self.root;
        loop {
            let mut next_node_ptr = next_node.load(Ordering::Acquire);
            if next_node_ptr.is_null() {
                match next_node.compare_exchange(
                    ptr::null_mut(),
                    new_node_ptr,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return,
                    Err(updated_ptr) => next_node_ptr = updated_ptr,
                }
            }

            let node = unsafe { &*next_node_ptr };
            if node.process < new_node.process {
                next_node = &node.left;
            } else {
                next_node = &node.right
            }
        }
    }

    /// Mutable version of `add`.
    pub(super) fn add_mut(&mut self, process: Pin<Box<ProcessData>>) {
        let new_node = Node::new(process);

        let mut node_ptr = self.root.get_mut();
        loop {
            // No processes in the node, we'll set ours.
            if node_ptr.is_null() {
                *node_ptr = Box::into_raw(new_node);
                return;
            }

            let node = unsafe { &mut **node_ptr };
            if node.process < new_node.process {
                node_ptr = node.left.get_mut();
            } else {
                node_ptr = node.right.get_mut();
            }
        }
    }

    /// Remove the next process to run from the queue.
    pub(super) fn remove(&self) -> Option<Pin<Box<ProcessData>>> {
        let root = self.root.load(Ordering::Acquire);

        // No processes ready to run.
        if root.is_null() {
            return None;
        }

        let mut parent_node = &self.root;
        let mut next_node = unsafe { &*root };
        loop {
            let next_left_node = next_node.left.load(Ordering::Acquire);
            if next_left_node.is_null() {
                // Return `next_node`.
                #[allow(trivial_casts)]
                match parent_node.compare_exchange(
                    next_node as *const _ as *mut _,
                    next_node.right.load(Ordering::Acquire),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(next_ptr) => {
                        let node = unsafe { Box::from_raw(next_ptr) };
                        return Some(node.process());
                    }
                    Err(updated_ptr) => {
                        next_node = unsafe { &*updated_ptr };
                        continue;
                    }
                }
            }

            parent_node = &next_node.left;
            next_node = unsafe { &*next_left_node };
        }
    }

    /// Mutable version of `remove`.
    pub(super) fn remove_mut(&mut self) -> Option<Pin<Box<ProcessData>>> {
        let root = self.root.get_mut();

        // No processes ready to run.
        if root.is_null() {
            return None;
        }

        let mut parent = root;
        loop {
            let node = unsafe { &mut **parent };
            let next_node = node.left.get_mut();
            if next_node.is_null() {
                let right = *node.right.get_mut();
                let node = unsafe { Box::from_raw(replace(parent, right)) };
                return Some(node.process());
            }

            parent = next_node;
        }
    }
}

impl Node {
    /// Returns a new `Node`.
    fn new(process: Pin<Box<ProcessData>>) -> Box<Node> {
        Box::new(Node {
            process,
            left: AtomicPtr::new(ptr::null_mut()),
            right: AtomicPtr::new(ptr::null_mut()),
        })
    }

    /// Unwrap the process from the node
    ///
    /// It drops the node in the process, **but not its left and right nodes**.
    fn process(self: Box<Self>) -> Pin<Box<ProcessData>> {
        // We can't just move `process` out of the `Node`, so we need to use
        // this trick.
        let process = unsafe { ptr::read(&self.process) };
        forget(self);
        process
    }
}

impl Drop for RunQueue {
    fn drop(&mut self) {
        let root = self.root.load(Ordering::Relaxed);
        if !root.is_null() {
            drop(unsafe { Box::from_raw(root) });
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let left = self.left.load(Ordering::Relaxed);
        if !left.is_null() {
            drop(unsafe { Box::from_raw(left) });
        }

        let right = self.right.load(Ordering::Relaxed);
        if !right.is_null() {
            drop(unsafe { Box::from_raw(right) });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;
    use std::pin::Pin;
    use std::time::Duration;

    use crate::rt::process::{Process, ProcessId, ProcessResult};
    use crate::rt::scheduler::{Priority, ProcessData};
    use crate::rt::RuntimeRef;

    use super::{Node, RunQueue};

    // TODO: concurrent testing.

    #[test]
    fn size_assertions() {
        assert_eq!(size_of::<RunQueue>(), 8);
        assert_eq!(size_of::<Node>(), 24);
    }

    struct TestProcess;

    impl Process for TestProcess {
        fn run(self: Pin<&mut Self>, _: &mut RuntimeRef, _: ProcessId) -> ProcessResult {
            ProcessResult::Complete
        }
    }

    fn add_process(run_queue: &RunQueue, fair_runtime: Duration) -> ProcessId {
        let process = Box::pin(ProcessData {
            priority: Priority::NORMAL,
            fair_runtime,
            process: Box::pin(TestProcess),
        });
        let pid = process.as_ref().id();
        run_queue.add(process);
        pid
    }

    fn add_process_mut(run_queue: &mut RunQueue, fair_runtime: Duration) -> ProcessId {
        let process = Box::pin(ProcessData {
            priority: Priority::NORMAL,
            fair_runtime,
            process: Box::pin(TestProcess),
        });
        let pid = process.as_ref().id();
        run_queue.add_mut(process);
        pid
    }

    /// Create a `RunQueue` test.
    macro_rules! runqueue_test {
        (
            // Name of the test.
            $name: ident,
            // List of processes to add in order. Value passed in the value of
            // the duration of the process' runtime (used in ordering in the
            // `RunQueue`).
            add_order: [ $($add: expr),* ],
            // The order in which the processes should be removed, 1-indexed.
            remove_order: [ $($remove: expr),* ],
        ) => {
            #[test]
            fn $name() {
                // Mutable version.
                let mut run_queue = RunQueue::empty();
                assert!(!run_queue.has_process());
                let pids = [
                    $( add_process_mut(&mut run_queue, Duration::from_secs($add)), )*
                ];
                assert!(run_queue.has_process());
                $( assert_eq!(run_queue.remove_mut().unwrap().as_ref().id(), pids[$remove - 1]); )*
                assert!(run_queue.remove_mut().is_none());

                // Immutable version
                let run_queue = RunQueue::empty();
                assert!(!run_queue.has_process());
                let pids = [
                    $( add_process(&run_queue, Duration::from_secs($add)), )*
                ];
                assert!(run_queue.has_process());
                $( assert_eq!(run_queue.remove().unwrap().as_ref().id(), pids[$remove - 1]); )*
                assert!(!run_queue.has_process());
                assert!(run_queue.remove().is_none());
            }
        };
    }

    #[test]
    fn tree_empty() {
        // (empty)
        let mut run_queue = RunQueue::empty();
        assert!(!run_queue.has_process());
        assert!(run_queue.remove_mut().is_none());
        assert!(run_queue.remove().is_none());
    }

    // Depth 1.

    runqueue_test!(
        tree_1_full,
        // 1
        add_order: [1],
        remove_order: [1],
    );

    // Depth 2.

    runqueue_test!(
        tree_2_right,
        // 1
        //  \
        //   2
        add_order: [1, 2],
        remove_order: [1, 2],
    );

    runqueue_test!(
        tree_2_left,
        //   1
        //  /
        // 2
        add_order: [2, 1],
        remove_order: [2, 1],
    );

    runqueue_test!(
        tree_2_full,
        //   1
        //  / \
        // 2   3
        add_order: [2, 1, 3],
        remove_order: [2, 1, 3],
    );

    // Depth 3, 3 nodes.

    runqueue_test!(
        tree_3_left_leaning,
        //     1
        //    /
        //   2
        //  /
        // 3
        add_order: [3, 2, 1],
        remove_order: [3, 2, 1],
    );

    runqueue_test!(
        tree_3_left_right,
        //   1
        //  /
        // 2
        //  \
        //   3
        add_order: [3, 1, 2],
        remove_order: [2, 3, 1],
    );

    runqueue_test!(
        tree_3_right_leaning,
        // 1
        //  \
        //   2
        //    \
        //     3
        add_order: [1, 2, 3],
        remove_order: [1, 2, 3 ],
    );

    runqueue_test!(
        tree_3_right_left,
        // 1
        //  \
        //   2
        //  /
        // 3
        add_order: [1, 3, 2],
        remove_order: [1, 3, 2],
    );

    // Depth 3, 4 nodes.

    runqueue_test!(
        tree_3_left_filled,
        //     1
        //    /
        //   2
        //  / \
        // 3   4
        add_order: [4, 2, 1, 3],
        remove_order: [3, 2, 4, 1],
    );

    runqueue_test!(
        tree_3_left_leaning_and_right,
        //     1
        //    / \
        //   2   3
        //  /
        // 4
        add_order: [3, 2, 1, 4],
        remove_order: [3, 2, 1, 4],
    );

    runqueue_test!(
        tree_3_left_right_and_right,
        //     1
        //    / \
        //   2   3
        //    \
        //     4
        add_order: [3, 1, 4, 2],
        remove_order: [2, 4, 1, 3],
    );

    runqueue_test!(
        tree_3_left_and_right_left,
        //     1
        //    / \
        //   2   3
        //      /
        //     4
        add_order: [2, 1, 4, 3],
        remove_order: [2, 1, 4, 3],
    );

    runqueue_test!(
        tree_3_right_leaning_and_left,
        //     1
        //    / \
        //   2   3
        //        \
        //         4
        add_order: [2, 1, 3, 4],
        remove_order: [2, 1, 3, 4],
    );

    runqueue_test!(
        tree_3_right_filled,
        // 1
        //  \
        //   2
        //  / \
        // 3   4
        add_order: [1, 3, 2, 4],
        remove_order: [1, 3, 2, 4],
    );

    // Depth 3, 5 nodes.

    runqueue_test!(
        tree_3_left_filled_and_right,
        //     1
        //    / \
        //   2   3
        //  / \
        // 4   5
        add_order: [4, 2, 5, 1, 3],
        remove_order: [4, 2, 5, 1, 3],
    );

    runqueue_test!(
        tree_3_left_leftand_right_left,
        //     1
        //    / \
        //   2   3
        //  /   /
        // 4   5
        add_order: [3, 2, 5, 1, 4],
        remove_order: [4, 2, 1, 5, 3],
    );

    runqueue_test!(
        tree_3_left_left_and_right_right,
        //     1
        //    / \
        //   2   3
        //  /     \
        // 4       5
        add_order: [3, 2, 4, 1, 5],
        remove_order: [4, 2, 1, 3, 5],
    );

    runqueue_test!(
        tree_3_left_right_and_right_left,
        //    1
        //  /   \
        // 2     3
        //  \   /
        //   4 5
        add_order: [3, 2, 5, 1, 4],
        remove_order: [4, 2, 1, 5, 3],
    );

    runqueue_test!(
        tree_3_left_right_and_right_right,
        //    1
        //  /  \
        // 2    3
        //  \    \
        //   4    5
        add_order: [3, 2, 4, 1, 5],
        remove_order: [4, 2, 1, 3, 5],
    );

    runqueue_test!(
        tree_3_left_and_right_filled,
        //   1
        //  / \
        // 2   3
        //    / \
        //   4   5
        add_order: [2, 1, 4, 3, 5],
        remove_order: [2, 1, 4, 3, 5],
    );

    // Depth 3, 6 nodes.

    runqueue_test!(
        tree_3_left_filled_and_right_left,
        //      1
        //    /   \
        //   2     3
        //  / \   /
        // 4   5 6
        add_order: [4, 2, 6, 1, 3, 5],
        remove_order: [4, 2, 5, 1, 6, 3],
    );

    runqueue_test!(
        tree_3_left_filled_and_right_right,
        //      1
        //    /   \
        //   2     3
        //  / \     \
        // 4   5     6
        add_order: [4, 2, 5, 1, 3, 6],
        remove_order: [4, 2, 5, 1, 3, 6],
    );

    runqueue_test!(
        tree_3_left_left_and_right_filled,
        //     1
        //    / \
        //   2   3
        //  /   / \
        // 4   5   6
        add_order: [3, 2, 5, 1, 4, 6],
        remove_order: [4, 2, 1, 5, 3, 6],
    );

    runqueue_test!(
        tree_3_left_right_and_right_filled,
        //    1
        //  /   \
        // 2     3
        //  \   / \
        //   4 5   6
        add_order: [3, 1, 5, 2, 4, 6],
        remove_order: [2, 4, 1, 5, 3, 6],
    );

    // Depth 3, 7 nodes.

    runqueue_test!(
        tree_3_full,
        //      1
        //    /   \
        //   2     3
        //  / \   / \
        // 4   5 6   7
        add_order: [4, 2, 6, 1, 3, 5, 7],
        remove_order: [4, 2, 5, 1, 6, 3, 7],
    );
}
