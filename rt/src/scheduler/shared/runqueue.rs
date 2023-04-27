use std::mem::replace;
use std::pin::Pin;
use std::sync::Mutex;

use crate::scheduler::shared::ProcessData;

// TODO: currently this creates and drops Node on almost every operation. Maybe
// we can keep (some of) the structure in place, changing `Node.process` into an
// Option as well?

/// Processes that are ready to run.
///
/// Implemented as a simple binary tree.
#[derive(Debug)]
pub(crate) struct RunQueue {
    root: Mutex<Branch>,
}

type Branch = Option<Box<Node>>;

#[derive(Debug)]
struct Node {
    process: Pin<Box<ProcessData>>,
    left: Branch,
    right: Branch,
}

impl RunQueue {
    /// Returns an empty `RunQueue`.
    pub(crate) const fn empty() -> RunQueue {
        RunQueue {
            root: Mutex::new(None),
        }
    }

    /// Returns the number of processes in the queue.
    ///
    /// # Notes
    ///
    /// Don't call this often, it's terrible for performance.
    pub(crate) fn len(&self) -> usize {
        match &mut *self.root.lock().unwrap() {
            Some(branch) => branch.len(),
            None => 0,
        }
    }

    /// Returns `true` if the queue contains any process.
    pub(crate) fn has_process(&self) -> bool {
        self.root.lock().unwrap().is_some()
    }

    /// Add `process` to the queue of running processes.
    pub(crate) fn add(&self, process: Pin<Box<ProcessData>>) {
        let mut next_node = &mut *self.root.lock().unwrap();
        loop {
            match next_node {
                Some(node) => {
                    // Select the next node in the branch to attempt to add
                    // ourselves to.
                    if node.process < process {
                        next_node = &mut node.left;
                    } else {
                        next_node = &mut node.right;
                    }
                }
                None => {
                    // Last node in the branch add our process to it.
                    *next_node = Some(Box::new(Node::new(process)));
                    return;
                }
            }
        }
    }

    /// Remove the next process to run from the queue.
    pub(crate) fn remove(&self) -> Option<Pin<Box<ProcessData>>> {
        let mut next_node = &mut *self.root.lock().unwrap();
        loop {
            match next_node {
                Some(node) if node.left.is_none() => {
                    // Reach the end of the left branch. Make the right branch
                    // the new parent node, ensuring its still part of the tree,
                    // and return the parent node's process.
                    let right_node = node.right.take();
                    return replace(next_node, right_node).map(|node| node.process);
                }
                Some(node) => {
                    // Another node on the left branch.
                    next_node = &mut node.left;
                }
                // This case can only happen on the root.
                None => return None,
            }
        }
    }
}

impl Node {
    /// Returns a new `Node`.
    const fn new(process: Pin<Box<ProcessData>>) -> Node {
        Node {
            process,
            left: None,
            right: None,
        }
    }

    /// Returns the number of processes in this node and it's descendants.
    fn len(&self) -> usize {
        let mut count = 1; // Count ourselves.
        if let Some(branch) = self.left.as_ref() {
            count += branch.len();
        }
        if let Some(branch) = self.right.as_ref() {
            count += branch.len();
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::mem::size_of;
    use std::pin::Pin;
    use std::task::{self, Poll};
    use std::time::Duration;

    use crate::process::{Process, ProcessId};
    use crate::spawn::options::Priority;

    use super::{Node, ProcessData, RunQueue};

    // TODO: concurrent testing.

    #[test]
    fn size_assertions() {
        assert!(size_of::<RunQueue>() <= 24);
        assert_eq!(size_of::<Node>(), 24);
    }

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

    fn add_process(run_queue: &RunQueue, fair_runtime: Duration) -> ProcessId {
        let mut process = Box::pin(ProcessData::new(Priority::NORMAL, Box::pin(TestProcess)));
        process.set_fair_runtime(fair_runtime);
        let pid = process.as_ref().id();
        run_queue.add(process);
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
                let run_queue = RunQueue::empty();
                assert!(!run_queue.has_process());
                let pids = [
                    $( add_process(&run_queue, Duration::from_secs($add)), )*
                ];
                assert!(run_queue.has_process());
                $(
                    let process = run_queue.remove().expect("failed to remove process");
                    assert_eq!(process.as_ref().id(), pids[$remove - 1]);
                )*
                assert!(!run_queue.has_process());
                assert!(run_queue.remove().is_none());
            }
        };
    }

    #[test]
    fn tree_empty() {
        // (empty)
        let run_queue = RunQueue::empty();
        assert!(!run_queue.has_process());
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
