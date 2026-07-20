# Changelog

All notable changes to `cargo-aiv`. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versioning is SemVer.

## [0.3.0]

### Added
- **Multi-function files.** Every top-level function in a file is now verified, not
  just the first — cargo-aiv works on real source files. Unsupported functions
  (methods, out-of-scope params) are skipped cleanly; their bodies stay so callees
  resolve.
- `--selftest` (trust guard), `--json` machine-readable output, `--help`/`--version`,
  and `--bound N` / `--unwind N` to tune BMC depth.
- End-to-end integration tests over the real binary (run under Kani in CI).

### Changed
- **`--json` output is now `{"results": [...], "summary": {...}}`** for both single
  files and batches (was a bare object for a single file). One result per function.

### Fixed
- **Unwinding-assertion failures were misreported as 🔴 BUG.** A loop exceeding the
  unwind bound is INCONCLUSIVE, not a panic — now classified correctly (raise
  `--unwind`). Found by fresh-corpus validation.
- **`Option<int>` overflow at `Some(i32::MAX)` was misreported as BUG** instead of
  UNGUARDED — the payload wasn't range-clamped in realistic mode. Fixed; a genuine
  `.unwrap()`-on-`None` still surfaces as a real BUG.
- Temp dirs are namespaced by PID + slot, so concurrent processes verifying
  same-named functions can't collide.

## [0.2.0]

The "usable on a real codebase" release. v0.1 verified one function for
panic-freedom; v0.2 proves arbitrary postconditions, covers the common AI-Rust
type shapes, and gates a whole directory in CI — without ever hanging or crying wolf.

### Added
- **`--prove '<expr>'`** — prove a postcondition over `result` (and the input
  names), e.g. `--prove 'result >= 0'`. `✅ PROVEN` (exit 0) / `🔴 VIOLATED`
  (exit 1, with witness) / `⏱️ INCONCLUSIVE` (exit 2). Turns the tool from a panic
  checker into a property verifier.
- **Batch mode** — `cargo-aiv src/*.rs` verifies every function in parallel,
  prints a summary table with inline bug witnesses, and exits 1 if any BUG.
  Concurrency capped at ~cores/4 (≤3) so heavy CBMC solvers don't starve each
  other past the timeout.
- **`⏱️ INCONCLUSIVE` verdict** — a per-mode 120s wall-clock cap (drained pipes,
  no deadlock) so a hard function degrades gracefully instead of hanging CI.
- **Slice params** — `&[int]`, `&mut [int]`, `&Vec<int>`; mutation through `&mut`
  is verified.
- **Tuple params** — `(i32, i32)`, … ; tuple returns work in both modes
  (`result.0`, `result.1`).
- **Readable counterexamples** — `0 (usize → empty vector)`, `i32::MIN`, etc.,
  instead of raw Kani tokens like `0ul`.
- **Unit tests** (11) covering value interpretation, playback parsing, and harness
  generation; `.github/workflows/ci.yml` (fmt/clippy/test/build) and `verify.yml`
  (the Kani + cargo-aiv gate).

### Fixed
- Counterexample parser merged tokens across Kani's multiple `concrete_vals`
  blocks (one per failing check), producing nonsensical witnesses in `--prove`.
  Now keeps only the first witness; regression-tested.

### Verified type coverage
Scalars (`i8..u64`, `bool`), `Vec<int>`, `Option<int>`, int slices, int tuples,
and any mix as a multi-param signature. Property: panic-freedom + integer
overflow, bounded (Vec ≤ 3, unwind ≤ 5).

## [0.1.0]

Initial release. Single-function panic-freedom verification with the dual-mode
(strict + realistic) classifier: `🔴 BUG` / `🟡 UNGUARDED` / `✅ VERIFIED`, built
on [Kani](https://github.com/model-checking/kani). Zero-annotation harness
synthesis from a `syn`-parsed signature.
