# Changelog

## 0.3.2

- `[verification] baseline_timeout_seconds` sets a separate wall-clock limit
  for a worker's clean baseline, which builds the worker's dependencies cold
  and can take far longer than one mutant. It defaults to `timeout_seconds`,
  the previous behavior.

## 0.3.1

- A failing or timed-out baseline now writes its whole output to
  `target/verus-mutants/logs/baseline-worker-N-<key>.log`, as a passing one
  always did. The error names the log and quotes the output from its first
  `error` line instead of the last 4000 bytes, which for Verus were often
  only trigger notes printed after the errors.

## 0.3.0

Verus support:

- Supports Verus 0.2026.08.30.b432e82 beside 0.2026.07.18.3a4d30b. The source
  parser is `verus_syn` 0.0.0-2026-08-02-0125 from crates.io (the release that
  ships with 0.2026.08.30), which parses both. `--version` lists the supported
  releases; the README has the table and how to add a release.
- Mutates the exec blocks of `open_atomic_invariant!` and
  `open_local_invariant!`, where atomic linearization points usually are.
  They were opaque macro bodies before.
- Verus output knowledge (proof diagnostics, summary lines, kill locations)
  lives in `src/adapters/`, one adapter per output generation behind one
  dispatch point. `strata.toml` enforces the module layering with
  `cargo strata check` in CI.

Containers and wrappers:

- `{source}` and `{target}` command placeholders: the worker's isolated source
  tree and Cargo target directory, for `docker run -v {source}:{source}`.
  The README documents a container setup.
- Fix: a timed-out oracle command now stops its whole process group
  (`SIGTERM`, then `SIGKILL` after 10 s) instead of killing only the direct
  child. Wrapper scripts and `docker run --init` containers no longer keep
  running and writing into the worker's target directory after a timeout.
  Processes a command leaves behind after it exits are stopped too, and
  Ctrl-C stops every running command.
- Fix: proof failures inside invariant blocks ("Cannot show invariant holds
  at end of block"), of logically atomic functions, of `no_unwind`, "loop
  invariant not satisfied" and every type-invariant failure are recognized
  kills with a location. The type-invariant kill kind is now
  `type-invariant` (was `other`).
- Fix: match-arm substitution skips bodies that name the other arm's
  bindings (`None => x`), which never compiled.
- Fix: a failure in a later crate's `verification results::` line is a proof
  kill. Only the first summary line was read, so a run that verified a
  dependency cleanly first and then failed was classified as invalid.

Since 0.2.0 (previously unreleased):

- Redundancy operators `drop-requires`, `drop-ensures` and `dead-refusal`
  (`--redundancy`), reported as findings rather than failures.
- `--baseline FILE`: accepted-survivor ratchet keyed by stable mutant ids.
- Kill locations (`kill` in `summary.json`): obligation kind, file, line and
  whether the failure was at an `ensures`.
- `{module}`, `{function}` and `{worker}` command placeholders; `{package}` in
  `baseline_command`.
- Ghost values (`let ghost`, `let tracked`, `Ghost(..)`, `Tracked(..)`) and
  `cfg(test)` / `cfg(feature)` code are not mutated.
- Struct-field substitution only swaps evidently same-typed values;
  statement deletion keeps the terminator; all Verus proof diagnostics are
  kills; composed verification test failures are `killed-by-test`.

## 0.2.0

Contract and specification operators, trusted-boundary challenges,
cargo-mutants inventory ingestion, diff and file scoping, isolated parallel
workers, mutation-score gates and deterministic reporting.

## 0.1.0

Initial release.
