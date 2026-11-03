verus! {

pub fn enq(q: &Queue, x: u64, Tracked(tok): Tracked<Token>, call: Tracked<CallToken>) -> (r: (u64, Tracked<CallToken>))
    atomically (au) {
        type EnqPred,
        (old: Observed) -> (commit: Out),
        requires
            old.id == q.id(),
        ensures
            commit@.0 == old.id + 1,
        outer_mask q.ns(),
        inner_mask none,
    },
    requires
        q.wf(),
    ensures
        r.0 == x,
{
    let mut v: u64 = 0;
    open_atomic_invariant!(q.inv.borrow() => g => {
        v = q.cell.load(Tracked(&g.perm));
        if v == 3 {
            q.cell.store(Tracked(&mut g.perm), v + 1);
        }
        proof {
            let tracked c = au.commit(1);
        }
    });
    let (s, call) = helper(q, call) atomically |update| {
        try_open_atomic_update_in_proof!(au, o => {
            let tracked c = update(o);
            Tracked(c)
        });
    };
    (x, call)
}

#[verifier::exec_allows_no_decreases_clause]
fn spin_wait(q: &Queue) {
    while q.flag.load() != 0
        invariant q.wf(),
    {
        q.hint();
    }
}

#[verifier::spinoff_prover]
fn heavy(a: u64, b: Ghost<int>) -> (r: u64)
    requires a < 10,
    ensures r == a + 1,
{
    a + 1
}

fn invariant_in_proof(q: &Queue) {
    proof {
        open_atomic_invariant_in_proof!(q.inv.borrow() => g => {
            g.x = 1;
        });
    }
    open_local_invariant!(q.local.borrow() => g => {
        q.cell.store(Tracked(&mut g.perm), 7);
    });
}
}

// Parse-only fixture for src/discover.rs: current Verus syntax around exec code.
// Line numbers are asserted by the test; append, do not insert.
