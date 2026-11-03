//! An even counter whose atomic cell is guarded by an atomic invariant. The
//! exec code inside `open_atomic_invariant!` is the object's linearization
//! point, so mutants there must be generated and checked by Verus.
use vstd::atomic::{PAtomicU64, PermissionU64};
use vstd::invariant::{AtomicInvariant, InvariantPredicate};
use vstd::open_atomic_invariant;
use vstd::prelude::*;

verus! {

pub struct EvenPred;

impl InvariantPredicate<int, PermissionU64> for EvenPred {
    closed spec fn inv(id: int, perm: PermissionU64) -> bool {
        perm.id() == id && perm.value() % 2 == 0
    }
}

pub struct Counter {
    pub cell: PAtomicU64,
    pub inv: Tracked<AtomicInvariant<int, PermissionU64, EvenPred>>,
}

impl Counter {
    pub closed spec fn wf(&self) -> bool {
        self.inv@.constant() == self.cell.id()
    }

    /// Stores `x`, rounded up to the next even number.
    pub fn store_even(&self, x: u64)
        requires
            self.wf(),
            x < 100,
    {
        open_atomic_invariant!(self.inv.borrow() => perm => {
            let y = if x % 2 == 0 { x } else { x + 1 };
            self.cell.store(Tracked(&mut perm), y);
        });
    }
}

} // verus!
