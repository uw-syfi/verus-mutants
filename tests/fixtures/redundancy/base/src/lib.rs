use vstd::prelude::*;

verus! {

// Findings: `x < 4000000000` is implied by `x < 2000000000`, and `r % 2 == 0` is used by no
// caller. Non-findings: `x < 2000000000` (the overflow check needs it) and
// `r == x * 2` (`app::scaled` relies on it).
pub fn double_small(x: u32) -> (r: u32)
    requires
        x < 2000000000,
        x < 4000000000,
    ensures
        r == x * 2,
        r % 2 == 0,
{
    x * 2
}

} // verus!
