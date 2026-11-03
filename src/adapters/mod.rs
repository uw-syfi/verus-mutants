//! The one dispatch point for Verus-release-specific knowledge of the
//! verifier's output. Everything else sees only the normalized answers below
//! (`KillLocation`, diagnostic kind labels, booleans) and never names an
//! adapter.
//!
//! One adapter per output-format generation lives in a child module. Verus
//! output carries no release number and the command that produced it may run
//! anywhere (a container, a remote host), so the dispatch does not detect a
//! release: every supported generation's patterns are accepted. A new
//! generation becomes a sibling adapter, and each function here consults every
//! adapter. Adapters never reference each other (`strata.toml` enforces it).
//!
//! Source syntax is version-sensitive too. `verus_syn` is the parser, and one
//! of its releases parses every supported Verus release (syntax only grows), so
//! discovery has a single parser; `strata.toml` confines `verus_syn` to
//! `discover.rs`.

mod v2026;

use crate::model::KillLocation;

/// A Verus release this tool is tested against.
pub struct Release {
    pub version: &'static str,
}

/// Supported Verus releases, oldest first. The README's table lists the same.
pub const SUPPORTED: &[Release] = &[
    Release {
        version: "0.2026.07.18.3a4d30b",
    },
    Release {
        version: "0.2026.08.30.b432e82",
    },
];

/// `--version` text: the tool version and the supported Verus releases.
pub fn long_version() -> String {
    let releases: Vec<&str> = SUPPORTED.iter().map(|release| release.version).collect();
    format!(
        "{}\nsupported Verus releases: {}",
        env!("CARGO_PKG_VERSION"),
        releases.join(", ")
    )
}

/// Kind label of the failed proof obligation in `output`, if Verus reported
/// one for a program that type-checked.
pub fn proof_failure(output: &str) -> Option<&'static str> {
    v2026::proof_failure(output)
}

/// Where the first failed proof obligation in `output` is.
pub fn kill_location(output: &str) -> Option<KillLocation> {
    v2026::kill_location(output)
}

/// Whether `output` shows Verus or rustc rejecting the program before
/// verification (a malformed mutant).
pub fn looks_invalid(output: &str) -> bool {
    v2026::looks_invalid(output)
}

/// Whether `output` reports at least one verified function and no errors.
pub fn has_positive_verification(output: &str) -> bool {
    v2026::has_positive_verification(output)
}
