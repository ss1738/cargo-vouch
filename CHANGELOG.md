# Changelog

All notable changes to `cargo-vouch`. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versioning is SemVer.

## [0.4.0]

### Added
- **Floating-point support (`f32`, `f64`).** Float params are now in scope, as scalars
  and inside tuples, `Option`, and same-file structs/enums. Floats get no realistic
  clamp: float arithmetic does not panic (it yields `inf`/`NaN`, not an overflow panic),
  so there is no BUG-vs-UNGUARDED split. Instead, Kani's default NaN / float-UB checks
  run, so a reachable NaN (e.g. `0.0 / 0.0` or `inf * 0.0` in a control loop) surfaces as
  a BUG. This makes cargo-vouch catch a real safety-critical hazard, not just panics.
  On the 26-function embedded corpus, combined with the 0.3.2 fix, definitive verdicts
  reached 26/26 (100%) with zero INCONCLUSIVE and zero UNSUPPORTED.

## [0.3.2]

### Fixed
- **Narrow integer types (`i8`, `u8`) now verify.** The realistic-mode bound clamped
  scalars to `±1000`, but that literal does not fit `i8` (-128..127) or `u8` (0..255),
  so those harnesses failed to compile and the function came back INCONCLUSIVE. Worse,
  because a file's harnesses compile together, one narrow-type function silently
  poisoned every other function in the same file (they all went INCONCLUSIVE too).
  `i8`/`u8` now skip the clamp (their whole domain is already within the realistic
  range). Measured on a 26-function embedded corpus this lifted definitive verdicts
  from 85% to 92% with zero INCONCLUSIVE (only genuine floats stay UNSUPPORTED).

## [0.3.0]

### Added
- **Multi-function files.** Every top-level function in a file is now verified, not
  just the first, cargo-vouch works on real source files. Unsupported functions
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
  unwind bound is INCONCLUSIVE, not a panic, now classified correctly (raise
  `--unwind`). Found by fresh-corpus validation.
- **`Option<int>` overflow at `Some(i32::MAX)` was misreported as BUG** instead of
  UNGUARDED, the payload wasn't range-clamped in realistic mode. Fixed; a genuine
  `.unwrap()`-on-`None` still surfaces as a real BUG.
- Temp dirs are namespaced by PID + slot, so concurrent processes verifying
  same-named functions can't collide.

## [0.2.0]

The "usable on a real codebase" release. v0.1 verified one function for
panic-freedom; v0.2 proves arbitrary postconditions, covers the common AI-Rust
type shapes, and gates a whole directory in CI, without ever hanging or crying wolf.

### Added
- **`--prove '<expr>'`**, prove a postcondition over `result` (and the input
  names), e.g. `--prove 'result >= 0'`. `✅ PROVEN` (exit 0) / `🔴 VIOLATED`
  (exit 1, with witness) / `⏱️ INCONCLUSIVE` (exit 2). Turns the tool from a panic
  checker into a property verifier.
- **Batch mode**, `cargo-vouch src/*.rs` verifies every function in parallel,
  prints a summary table with inline bug witnesses, and exits 1 if any BUG.
  Concurrency capped at ~cores/4 (≤3) so heavy CBMC solvers don't starve each
  other past the timeout.
- **`⏱️ INCONCLUSIVE` verdict**, a per-mode 120s wall-clock cap (drained pipes,
  no deadlock) so a hard function degrades gracefully instead of hanging CI.
- **Slice params**, `&[int]`, `&mut [int]`, `&Vec<int>`; mutation through `&mut`
  is verified.
- **Tuple params**, `(i32, i32)`, … ; tuple returns work in both modes
  (`result.0`, `result.1`).
- **Readable counterexamples**, `0 (usize → empty vector)`, `i32::MIN`, etc.,
  instead of raw Kani tokens like `0ul`.
- **Unit tests** (11) covering value interpretation, playback parsing, and harness
  generation; `.github/workflows/ci.yml` (fmt/clippy/test/build) and `verify.yml`
  (the Kani + cargo-vouch gate).

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
