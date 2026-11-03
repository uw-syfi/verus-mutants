# cargo-verus-mutants

`cargo-verus-mutants` is an experimental mutation runner for Verus Cargo
projects. It combines source-aware automatic mutations in executable Verus
code with manually specified domain mutations.

The automatic campaign is zero-config. From a Verus workspace or any member
crate, run:

```sh
cargo verus-mutants run
```

The tool resolves the workspace, finds Verus packages from existing Cargo
metadata or `verus!` source, and verifies each package with
`cargo verus build -p {package}`. `.verus-mutants.toml` is optional.

## Supported Verus releases

| Verus release | Source parser | Output adapter |
| --- | --- | --- |
| 0.2026.07.18.3a4d30b | `verus_syn` 0.0.0-2026-08-02-0125 | `adapters::v2026` |
| 0.2026.08.30.b432e82 | `verus_syn` 0.0.0-2026-08-02-0125 | `adapters::v2026` |

`cargo verus-mutants --version` prints the same list. Both releases are
covered by the Verus end-to-end tests (see Development). Verus syntax only
grows, so one `verus_syn` release parses every supported release. Verus
output carries no release number, so every supported output adapter's
patterns are accepted; see "Adding a Verus release" below.

## Install and use

```sh
cargo install cargo-verus-mutants
cd my-verus-workspace

# Inspect the deterministic mutant inventory without running Verus.
cargo verus-mutants list

# Run the automatic executable-code campaign.
cargo verus-mutants run

# Sample ten automatically generated executable-code mutants.
cargo verus-mutants run --automatic-only --limit 10
```

The tool can also run without installation:

```sh
git clone https://github.com/uw-syfi/verus-mutants
cargo install --path verus-mutants
```

Each oracle first runs against one clean isolated source tree. Mutants are
applied and restored one at a time in that tree, reusing its Cargo target so
incremental Verus builds remain effective. Logs and an atomic JSON summary are
written to `target/verus-mutants/` in the analyzed project.
The process exits unsuccessfully for survivors, timeouts, or infrastructure
failures, after publishing the report. Invalid mutants remain visible but do
not fail the campaign.

## Layout

The engine is independent of the workspace it analyzes:

```text
src/cargo.rs        Cargo metadata and verified-package discovery
src/config.rs       .verus-mutants.toml schema
src/discover.rs     Verus parsing and automatic operators
src/materialize.rs  isolated copies and guarded source edits
src/oracle.rs       oracle commands and result classification
src/adapters/       Verus output knowledge, one adapter per output generation
src/process.rs      process groups, timeouts and interrupts
src/runner.rs       baselines, selection, and campaign orchestration
src/report.rs       terminal and JSON reports
```

`strata.toml` states the module layering (for example: only `oracle`
consults the adapters, only through `adapters`; adapters never reference each
other; only `discover.rs` uses `verus_syn`; only `process.rs` waits on child
processes). `cargo strata check` enforces it in CI.

Projects use `.verus-mutants.toml` only for overrides or curated domain
mutants.

## Verifying in a container

Verus often runs in a container image (its release binaries need a recent
glibc, and a pinned image keeps the toolchain reproducible). The verification
command can be any program, so a `docker run` works directly. Each worker
runs the command in its isolated copy of the workspace (`{source}`) with its
own Cargo target directory (`{target}`); mount both at the same paths so the
paths in Verus diagnostics are the paths the tool knows:

```toml
[verification]
command = [
  "docker", "run", "--rm", "--init", "--user", "1000:1000",
  "-v", "{source}:{source}", "-v", "{target}:{target}", "-w", "{source}",
  "-e", "CARGO_TARGET_DIR={target}", "-e", "HOME=/tmp",
  "-e", "CARGO_HOME=/cargo", "-v", "/path/to/cargo-cache:/cargo",
  "my-verus-image:tag", "cargo", "verus", "build", "-p", "{package}",
]
timeout_seconds = 600
```

Or keep the container details in a script in the repository; it is copied
into each worker's tree and runs there:

```sh
#!/bin/sh
# verify: run "$@" in the Verus image, current directory and target mounted.
exec docker run --rm --init --user "$(id -u):$(id -g)" -e HOME=/tmp \
  -e CARGO_HOME=/cargo -v "$HOME/.cache/verus-cargo:/cargo" \
  -v "$PWD:$PWD" -w "$PWD" \
  -e CARGO_TARGET_DIR -v "$CARGO_TARGET_DIR:$CARGO_TARGET_DIR" \
  my-verus-image:tag "$@"
```

```toml
[verification]
command = ["./verify", "cargo", "verus", "build", "-p", "{package}"]
```

Notes:

- `--init` matters. On a timeout the tool sends `SIGTERM` to the command's
  whole process group, which `docker run` forwards to the container, and
  `SIGKILL` 10 seconds later. Without an init process, the container's first
  process ignores `SIGTERM`, the container outlives the `docker` client, and
  it keeps writing into the worker's target directory. Processes a command
  leaves running in its group after it exits are stopped too, and Ctrl-C
  stops every running command.
- A host directory the user owns (a named volume is owned by root) as
  `CARGO_HOME` keeps the downloaded `vstd` between runs. Each worker's first build compiles and verifies the
  dependencies once; later mutants rebuild only the mutated package.
- Worker trees live in the system temporary directory; set `TMPDIR` to put
  them elsewhere (for example on a file system the container host shares).
- The image needs `cargo verus` and the Rust toolchain that Verus release
  wants. The crates need `vstd` (and `verus_builtin` and
  `verus_builtin_macros` if they use macros such as
  `try_open_atomic_update!` that expand to them) at the image's Verus
  release, plus `[package.metadata.verus] verify = true`.

Workspaces that are not Cargo packages can still use the tool through a
script that builds them its own way: discovery needs the Cargo workspace to
find packages and sources, and the command decides how to verify.

## Configuration

```toml
[project]
include_packages = ["verified-crate"]
exclude_globs = ["target/**", "generated/**"]
exclude_functions = ["ffi_*", "trusted_boundary"]

[verification]
command = ["cargo", "verus", "build", "-p", "{package}"]
baseline_command = ["cargo", "verus", "build", "--workspace"]
timeout_seconds = 240

# Command placeholders (verification.command, baseline_command and command
# oracles): {package}; {module}, the Verus module path of the mutated file
# below src/ (a/b.rs gives a::b, lib.rs gives the crate name); {function}, the
# mutated function; {worker}, the zero-based --jobs worker index; {source} and
# {target}, the worker's isolated source tree (the command's working
# directory) and Cargo target directory (also in CARGO_TARGET_DIR). For example
#   command = ["env", "TARGET_VOL=vol-{worker}", "./verify", "cargo", "verus",
#              "build", "-p", "{package}", "--", "--verify-module", "{module}"]

[operators]
# Optional assurance-hardening campaigns. Disabled by default because these
# mutate the oracle itself rather than only executable implementation code.
mutate_contracts = false
mutate_spec_functions = false
condition_to_true = true
condition_to_false = true
logical_clause_deletion = true
relational_replacement = true
boolean_literal_replacement = true
integer_literal_replacement = true
arithmetic_replacement = true
statement_deletion = true
struct_field_value_substitution = true
match_arm_body_substitution = true
external_body_insertion = false
external_body_visibility_widening = false

# Route trust-boundary challenges to a structural policy oracle instead of
# treating a successful Verus run as survival.
[operator_oracles.insert-external-body]
kind = "command"
command = ["python3", "ci/check_verified_architecture.py"]
expected_pattern = "verified architecture policy was violated"

# Reuse cargo-mutants for ordinary Rust syntax while retaining this runner's
# custom oracle and outcome classification.
[rust_mutants]
enabled = true
inventory_command = ["cargo", "mutants", "--list", "--json", "--workspace"]

[rust_mutants.oracle]
kind = "command"
command = ["python3", "ci/check_rust_mutant.py", "{package}"]
expected_pattern = "AUTOMATIC_MUTANT_REJECTED"
invalid_pattern = "AUTOMATIC_MUTANT_INVALID"

[[manual_mutant]]
id = "M-DOMAIN-FAULT"
file = "crates/verified-crate/src/lib.rs"
replace = "old source"
with = "mutated source"
```

The manual mutant's package is inferred from its file and its oracle defaults to
Verus. A nonstandard test or command oracle can set `kind`, `command`,
`required_test_count`, and `expected_pattern` under `[manual_mutant.oracle]`.

## Mutation and oracle semantics

Ghost code is never mutated: `let ghost`, `let tracked`, `Ghost<T>` and
`Tracked<T>` bindings, `Ghost(..)` and `Tracked(..)` calls, `proof` blocks, and
spec and proof functions (the last two only when `mutate_spec_functions` is
off). Items gated by `#[cfg(test)]` or `#[cfg(feature = "...")]`
(including `all(..)` containing one, or an `any(..)` whose alternatives are all
gated) are skipped, since the default build does not verify them.

The automatic campaign parses `verus!` bodies with `verus_syn`. By default it
visits only default or `exec` function bodies, including the blocks of
`open_atomic_invariant!` and `open_local_invariant!`, which are exec code
(usually an object's atomic linearization point). Other macro bodies,
including `open_atomic_invariant_in_proof!` and the atomic-update macros, are
not mutated, nor are the `atomically` specifications of logically atomic
functions. It does not mutate assertions,
assumptions, quantifiers, proof closures, or loop proof clauses. Operators cover
conditions, logical clauses, relational and arithmetic operators, literals,
standalone effect statements (`;`-terminated calls and assignments), struct
field values, and match-arm bodies. Struct-field substitution swaps a field's
value with its successor's only when both are literals of one kind and suffix
or casts to the same type, because the tool has no type information; shorthand
fields are skipped. Match-arm substitution moves each arm's body into the
previous arm, unless the body names a binding of its own pattern that the
receiving pattern does not bind.

`mutate_contracts` and `mutate_spec_functions` enable a separate assurance
hardening campaign. These mutations challenge whether the rest of the proof
actually depends on a precondition, invariant, or model relation. They mutate
the oracle itself, so projects should review survivors as specification gaps,
not ordinary implementation-test gaps.

`external_body_insertion` challenges the trusted boundary automatically.
Projects normally route that operator to an architecture-policy command through
`operator_oracles`, as shown above. The repository-specific input is the trust
policy and its oracle, not a textual mutant.

`external_body_visibility_widening` makes each non-public `external_body`
function public. Route it to the same architecture-policy oracle to check that
foreign-effect settlement functions cannot be exported accidentally.
Bodies of existing `external_body` functions are not mutated because Verus
deliberately does not verify them. Their contracts are still mutated when
`mutate_contracts` is enabled.

`rust_mutants` accepts the JSON inventory emitted by cargo-mutants and converts
it into the same isolated mutation and oracle protocol. This lets projects use
cargo-mutants' mature ordinary-Rust discovery without forcing `cargo test` to
be the only oracle. The inventory command, production feature matrix, and
architecture oracle remain project inputs because a generic mutation engine
cannot infer those policies.

Use repeated `--operator` arguments to focus an operator class, or
`--limit-per-operator N` for a deterministic sample from every package/operator
pair. Repeat `--exhaustive-operator NAME` to keep every mutant for a small,
security-critical operator while sampling the rest of the campaign.
By default any survivor fails the command. `--minimum-kill-rate RATE` enables
the conventional mutation-score mode for broad automatic campaigns, where
equivalent mutants are expected. Invalid mutants are excluded from the rate;
timeouts and infrastructure failures always fail.
Repeat `--require-zero-survivors-for OPERATOR` to keep a critical operator at
100% even when the overall campaign uses a mutation-score threshold.
`--in-diff REF` limits discovery to files changed since `merge-base REF HEAD`,
including uncommitted and untracked files. Operators named by
`--exhaustive-operator` remain repository-wide, which keeps small trust-boundary
campaigns exhaustive while ordinary PR mutations stay diff-scoped.
Use repeated `--file PATH` arguments when a container or remote runner cannot
read the repository's Git metadata. Paths are workspace-relative and have the
same exhaustive-operator behavior as `--in-diff`.
`--jobs N` runs N isolated workers. Each worker owns its source and Cargo target
directories, so concurrent mutations cannot share edited source or stale build
artifacts. Baselines are repeated per worker to preserve that isolation.

### Redundancy campaign

Three more operators turn the verifier into a redundancy detector. For them a
mutant that still verifies is a finding about the specification or code, not a
weak check, so it is reported as `redundant` in its own "Redundant" report
section and never fails the run (survivors stay in "Survivors"). They are off
by default. `--redundancy` enables all three; `--operator NAME` enables just
that one (or set `drop_requires`, `drop_ensures`, `dead_refusal` under
`[operators]`).

| Operator | Edit | Redundant means |
| --- | --- | --- |
| `drop-requires` | delete one `requires` clause of a function | the function and all its callers verify without it: the interface is over-constrained |
| `drop-ensures` | delete one `ensures` clause | every caller verifies without it: no caller uses the guarantee |
| `dead-refusal` | insert `assert(false);` at the start of a branch whose own statements `return` a refusal | the branch is unreachable: the run-time check can become a proof |

`drop-*` apply to exec and proof functions, including trait methods. A sole
clause is deleted with its keyword. `dead-refusal` looks at `if` and `else`
blocks and `match` arms (not loops); a `return` expression is a refusal if its
text, ignoring whitespace, contains one of `[operators] refusal_patterns`
(default `return Err(`, `return None`; add named refusal enums such as
`return Refusal::`).

Callers can live anywhere, so a redundancy mutant is verified package by
package: the defining package first, then every Verus workspace package that
transitively depends on it (nearest first), stopping at the first failure. The
mutant is redundant only if all verify, and each finding lists the packages
that were re-verified. The command is `verification.redundancy_command`, else
`baseline_command`, else `command`, with `{package}` set per package. It must
verify the whole package: do not scope it with `{module}`, since callers are in
other modules. Each package is also verified clean once per worker as a
baseline. Sampling (`--limit-per-operator`, `--limit`), `--in-diff`, `--file`,
`--jobs` and `--operator` apply as for other operators; the baseline file only
lists survivors. Each finding also carries `detail` in `summary.json` (the
clause text or branch condition) and `verified_packages`.

A finding is a prompt to look, not an instruction to delete: an unused
`ensures` may document an API, and a dead refusal may guard against a future
caller. Cost per mutant is one whole-package verification per re-verified
package, so it is higher than a module-scoped exec mutant.

### Accepted-survivor baseline

`--baseline FILE` (TOML, or JSON when the name ends in `.json`) lists survivors
the project accepts. With it, a run fails only on survivors that are not
listed, on listed entries whose mutants ran and no longer survive, and on
entries that match no discovered mutant (a ratchet: fixed gaps and renamed code
must leave the file). Entries outside the current `--file`, `--in-diff` or
limit scope are ignored. `--minimum-kill-rate` and
`--require-zero-survivors-for` do not count accepted survivors.

```toml
[[mutant]]
file = "crates/x/src/pool.rs"          # workspace-relative
function = "alloc"                      # omit for manual mutants
operator = "condition-to-true"
replacement = "true"
status = "equivalent"                   # or "open"
reason = "the guard is implied by the loop invariant"   # required for equivalent
# owner = "alice"                       # required for open
# original = "n > 0"                    # optional: disambiguates same-key mutants
```

JSON uses the same fields under a top-level `"mutants"` array.

The stable id is the tuple `(file, function, operator, replacement text)`.
Line numbers, byte offsets and the `VM-` hash are excluded, so unrelated edits
do not invalidate entries. An entry covers every mutant with that key in the
function (add `original` to pick one); changing the mutated source text, the
function name, or the file moves the mutant and the entry becomes "no longer
exists". Unlisted survivors are printed as ready-to-edit `[[mutant]]` blocks.

Results have distinct meanings:

- `killed-by-proof`: Verus rejected a well-formed mutant with a recognized
  proof diagnostic.
- `killed-by-test` or `killed-by-policy`: a configured dynamic or structural
  oracle rejected it.
- `survived`: the oracle accepted the mutant. This is the result to inspect.
- `redundant`: a redundancy-operator mutant that still verified (see above).
- `invalid`: the edit did not produce a type-correct, supported Verus program.
- `timeout`: inconclusive.
- `infrastructure-failure`: the oracle failed without its expected rejection.

Each killed-by-proof result in `summary.json` carries a `kill` object with the
failed obligation's `kind`, `file`, `line`, `column`, and `from_ensures` (true
when the error is at a postcondition rather than an in-body assert, invariant,
call precondition, or arithmetic check). The kinds are `postcondition`,
`precondition`, `invariant` (loop invariants and invariant blocks), `atomic`,
`assertion`, `decreases`, `division`, `arithmetic`, `bounds`, `unwind`
and `type-invariant`. A failure Verus counts in its summary line under a
diagnostic the tool does not know is still a kill, without a `kill` object.
The terminal report counts both.

Invalid mutants and timeouts are not kills. A clean Verus baseline must report
at least one verified function and zero errors. A focused test baseline must
pass at least `required_test_count` tests, or one test by default.

## MVP limits

This is source-aware, not compiler-native. `verus_syn` can identify syntactic
exec regions, but only Verus can resolve all expression modes and types. The
runner therefore filters invalid mutants after generation. Each worker is
sequential and reuse one isolated incremental target. Use package filters,
operator filters, mutant IDs, and deterministic limits for development runs.

The next scalability step is a Verus-side mutation inventory containing stable
IR node IDs, resolved modes and types, followed by process-level parallelism
and content-addressed baseline/build caches. The configuration, oracle, and
report schema can remain the external interface for that implementation.

## Development

```sh
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo install cargo-strata --version 0.4.0 --locked
cargo strata check
```

The end-to-end tests (`tests/*_e2e.rs`) run real Verus and are ignored by
default. Run them once per supported release, with an executable that runs
`cargo verus build "$@"` (for example the `verify` script above with `cargo
verus build` appended):

```sh
VERUS_FIXTURE_VERIFY=./verify-0.2026.07.18 cargo test -- --ignored
VERUS_FIXTURE_VERIFY=./verify-0.2026.08.30 \
  VERUS_FIXTURE_VSTD='"=0.0.0-2026-08-30-0159"' cargo test -- --ignored
```

The fixtures depend on the oldest supported release's `vstd`;
`VERUS_FIXTURE_VSTD` replaces that dependency for another release.

### Adding a Verus release

1. Run the end-to-end tests against it (above). If they pass and
   `cargo verus-mutants list` reports no parse warnings on real code, add the
   release to `adapters::SUPPORTED` and the table at the top.
2. If its syntax needs a newer parser, raise `verus_syn` to the release that
   ships with it (the `dependencies/syn` version in Verus at that commit; it is
   published on crates.io) and check that the older releases' fixtures still
   parse.
3. If its diagnostics or summary lines change, add an adapter beside
   `adapters/v2026.rs` for the new patterns and consult it from the functions
   in `adapters/mod.rs`. Keep the old adapter for the older releases.

Licensed under either the Apache License, Version 2.0 or the MIT License, at
your option.
