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
src/oracle.rs       process execution and result classification
src/runner.rs       baselines, selection, and campaign orchestration
src/report.rs       terminal and JSON reports
```

Projects use `.verus-mutants.toml` only for overrides or curated domain
mutants.

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

The automatic campaign parses `verus!` bodies with `verus_syn`. By default it
visits only default or `exec` function bodies. It does not mutate assertions,
assumptions, quantifiers, proof closures, or loop proof clauses. Operators cover
conditions, logical clauses, relational and arithmetic operators, literals,
standalone effect statements, struct field values, and match-arm bodies.

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

Results have distinct meanings:

- `killed-by-proof`: Verus rejected a well-formed mutant with a recognized
  proof diagnostic.
- `killed-by-test` or `killed-by-policy`: a configured dynamic or structural
  oracle rejected it.
- `survived`: the oracle accepted the mutant. This is the result to inspect.
- `invalid`: the edit did not produce a type-correct, supported Verus program.
- `timeout`: inconclusive.
- `infrastructure-failure`: the oracle failed without its expected rejection.

Invalid mutants and timeouts are not kills. A clean Verus baseline must report
at least one verified function and zero errors. A focused test baseline must
pass at least `required_test_count` tests, or one test by default.

## MVP limits

This is source-aware, not compiler-native. `verus_syn` can identify syntactic
exec regions, but only Verus can resolve all expression modes and types. The
runner therefore filters invalid mutants after generation. Campaigns are
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
```

Licensed under either the Apache License, Version 2.0 or the MIT License, at
your option.
