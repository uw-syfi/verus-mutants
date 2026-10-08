use base::double_small;
use vstd::prelude::*;

verus! {

pub fn scaled(x: u32) -> (y: u32)
    requires
        x < 100,
    ensures
        y == 2 * x,
{
    double_small(x)
}

// Used by `scaled_twice`, so `scaled`'s ensures is a non-finding; the ensures
// of `scaled_twice` itself has no caller, a finding.
pub fn scaled_twice(x: u32) -> (y: u32)
    requires
        x < 50,
    ensures
        y == 4 * x,
{
    scaled(x) * 2
}

// Finding: the refusal is unreachable under the precondition. The
// precondition itself is needed: `r is Ok` and `x + 1` depend on it.
pub fn checked_dead(x: u32) -> (r: Result<u32, ()>)
    requires
        x < 100,
    ensures
        r is Ok,
{
    if x >= 100 {
        return Err(());
    }
    Ok(x + 1)
}

// Non-finding: with no precondition the refusal is reachable.
pub fn checked_live(x: u32) -> (r: Result<u32, ()>) {
    if x >= 100 {
        return Err(());
    }
    Ok(x + 1)
}

} // verus!
