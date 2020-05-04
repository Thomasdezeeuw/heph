use std::mem::replace;
use std::pin::Pin;

use crate::rt::scheduler::{ProcessData, ProcessId};
use crate::util::{Either, TaggedBox};

/// Number of branches per level of the tree, must be a power of 2.
// TODO: benchmark a size of 2, only using 2 bits at a time. This would increase
// the maximum depth of the tree to ~64 (from ~32).
const N_BRANCHES: usize = 4;
/// Number of bits to shift per level.
const LEVEL_SHIFT: usize = N_BRANCHES / 2;
/// Number of bits to mask per level.
const LEVEL_MASK: usize = (1 << LEVEL_SHIFT) - 1;
/// For alignment reasons the two least significant bits are always 0, so we can
/// safely skip them.
// FIXME: add assertion test for this.
const SKIP_BITS: usize = 2;

/// Returns `false` if `ptr`'s `SKIP_BITS` aren't `0`.
pub(super) fn ok_ptr(ptr: *const ()) -> bool {
    ptr as usize & ((1 << SKIP_BITS) - 1) == 0
}

/// Inactive processes.
///
/// Implemented as a tree with four pointers on each level. A pointer can point
/// to a `Branch`, which again contains four pointers, or point to
/// `ProcessData`.
///
/// Because processes should have short ready state times (see process states),
/// but longer total lifetime they quickly move into and out from the structure.
/// To ensure operations remain quick we keep the structure of tree in place
/// when removing processes.
// pub(crate) because its used in AddActor.
pub(crate) struct Inactive {
    root: Branch,
    length: usize,
}

/// Allocated `Branch` or `ProcessData`.
type TreeBox = TaggedBox<Branch, ProcessData>;

struct Branch {
    branches: [Option<TreeBox>; N_BRANCHES],
}

impl Inactive {
    /// Create an empty `Inactive` tree.
    pub(super) const fn empty() -> Inactive {
        Inactive {
            root: Branch::empty(),
            length: 0,
        }
    }

    /// Returns `true` if the tree has any processes, `false` otherwise.
    pub(super) fn has_process(&self) -> bool {
        self.length > 0
    }

    /// Add a `process`.
    pub(super) fn add(&mut self, process: Pin<Box<ProcessData>>) {
        let pid = process.as_ref().id();
        self.root.add(process, pid.0 >> SKIP_BITS, 1);
        self.length += 1;
    }

    /// Removes the process with id `pid`, if any.
    pub(super) fn remove(&mut self, pid: ProcessId) -> Option<Pin<Box<ProcessData>>> {
        self.root.remove(pid, pid.0 >> SKIP_BITS).map(|process| {
            debug_assert_eq!(process.as_ref().id(), pid);
            self.length -= 1;
            process
        })
    }
}

impl Branch {
    const fn empty() -> Branch {
        Branch {
            branches: [None, None, None, None],
        }
    }

    pub(super) fn add(&mut self, process: Pin<Box<ProcessData>>, w_pid: usize, depth: usize) {
        match &mut self.branches[w_pid & LEVEL_MASK] {
            Some(branch) => match branch.deref_mut() {
                Either::Left(mut branch) => branch.add(process, w_pid >> LEVEL_SHIFT, depth + 1),
                Either::Right(_) => {
                    let new_branch = TreeBox::new_left(Box::pin(Branch::empty()));

                    let other_process =
                        replace(&mut self.branches[w_pid & LEVEL_MASK], Some(new_branch))
                            .map(|process| process.into_inner().unwrap_right())
                            .unwrap();

                    self.branches[w_pid & LEVEL_MASK]
                        .as_mut()
                        .unwrap()
                        .deref_mut()
                        .unwrap_left()
                        .add_both(process, other_process, w_pid >> LEVEL_SHIFT, depth)
                }
            },
            branch @ None => *branch = Some(TreeBox::new_right(process)),
        }
    }

    /// Add two process at a time.
    ///
    /// Used when a process is added but another process is encountered. Instead
    /// of adding one process (by creating a new level) and then add the other,
    /// possibly creating another conflict. We first create enough levels to not
    /// have conflicting branches and then add both processes.
    ///
    /// # Notes
    ///
    /// `self` must be empty.
    pub(super) fn add_both(
        &mut self,
        process1: Pin<Box<ProcessData>>,
        process2: Pin<Box<ProcessData>>,
        w_pid: usize,
        depth: usize,
    ) {
        debug_assert!(match self.branches {
            [None, None, None, None] => true,
            _ => false,
        });

        let pid1 = process1.as_ref().id();
        let pid2 = process2.as_ref().id();

        // Create enough branches to ensure when adding the process again
        // they won't conflict.
        let mut node: &mut Branch = &mut *self;
        let mut w_pid = w_pid;

        // TODO: do this in reverse: create the lowest first so we don't have to
        // deal with the `TreeBox` variants.
        for _ in 0..=diff_branch_depth(pid1, pid2, depth) {
            match &mut node.branches[w_pid & LEVEL_MASK] {
                branch @ None => {
                    *branch = Some(TreeBox::new_left(Box::pin(Branch::empty())));
                    node = branch.as_mut().unwrap().deref_mut().unwrap_left().get_mut();
                }
                _ => unreachable!(),
            }

            w_pid >>= LEVEL_SHIFT;
        }

        self.add(process1, skip_bits(pid1, depth), depth);
        self.add(process2, skip_bits(pid2, depth), depth);
    }

    pub(super) fn remove(&mut self, pid: ProcessId, w_pid: usize) -> Option<Pin<Box<ProcessData>>> {
        match self.branches[w_pid & LEVEL_MASK]
            .as_mut()
            .map(|b| b.deref_mut())
        {
            Some(Either::Left(mut next_branch)) => next_branch.remove(pid, w_pid >> LEVEL_SHIFT),
            Some(Either::Right(process)) if process.as_ref().id() == pid => self.branches
                [w_pid & LEVEL_MASK]
                .take()
                .map(|process| process.into_inner().unwrap_right()),
            Some(Either::Right(_)) | None => None,
        }
    }
}

/// Returns the depth at which `pid1` and `pid2` use difference branches.
///
/// # Notes
///
/// This loops forever if `pid1` and `pid2` are equal.
fn diff_branch_depth(pid1: ProcessId, pid2: ProcessId, depth: usize) -> usize {
    debug_assert!(pid1 != pid2);
    let mut pid1 = skip_bits(pid1, depth);
    let mut pid2 = skip_bits(pid2, depth);
    debug_assert!(pid1 != pid2);
    let mut depth = 0;
    while pid1 & LEVEL_MASK == pid2 & LEVEL_MASK {
        debug_assert!(depth < 32);
        debug_assert!(pid1 > 0);
        depth += 1;
        pid1 >>= LEVEL_SHIFT;
        pid2 >>= LEVEL_SHIFT;
    }
    depth
}

/// Skip the bits up to `depth`.
fn skip_bits(pid: ProcessId, depth: usize) -> usize {
    pid.0 >> ((depth * LEVEL_SHIFT) + SKIP_BITS)
}
