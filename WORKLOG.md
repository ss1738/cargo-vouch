# cargo-aiv — Work Log

## Week 1 — Spike / de-risk (goal: does Kani actually catch bugs in AI-generated Rust, fast?)

### Day 1 — 2026-07-20  ✅ CORE RISK DE-RISKED

**Done:**
- Project set up at `~/cargo-aiv/` (`corpus/`, `harnesses/`, `spike/`).
- Toolchain: Rust 1.94 present; **Kani 0.67.0 installed** (`cargo install --locked kani-verifier` + `cargo-kani setup`, ~first-time CBMC download).
- **Corpus generated** (`corpus/*.rs`, `corpus/manifest.json`): 12 AI-authored Rust utility fns via GPT-4o — 6 seeded-buggy (empty-vec `unwrap`, div-by-zero, overflow), 6 "correct". Realistic Copilot-style code.
- **Hand-written Kani harnesses** for 3 (`harnesses/examples.rs`) — the pattern the generator will auto-produce (signature → bounded symbolic inputs, `unwind(5)`).
- **Ran `cargo kani` on the AI code.** Result:

| Harness | AI's label | Kani verdict | Bug found |
|---|---|---|---|
| `verify_divide` | buggy | **FAILED** | divide-by-zero **+** `i32::MIN / -1` overflow |
| `verify_sum_vec` | **"correct"** | **FAILED** | **add-with-overflow** |
| `verify_find_max` | buggy | **FAILED** | `Option::unwrap()` on `None` (empty vec) |

`Complete - 0 successfully verified, 3 failures, 3 total.` Each ran in **<1s**.

**The result that matters:** `sum_vec` was labelled *correct* by the AI and would **pass `cargo test`**, but Kani **proved it overflows** (`[i32::MAX, 1]`). That single example is the product thesis, the demo, and exhibit A of "The Panic Log" — all on day 1.

**Risk status:**
- ✅ "Kani chokes on AI Rust / too slow" — **cleared for these shapes** (<1s each). Kill criterion (>60% verify in <60s) already met.
- ⏳ still to test wk 1–2: iterator-heavy fns (`.iter().map().filter().collect()`) — the real timeout risk.

**Next (Week 1 remainder):**
1. Run the full 12-fn corpus through Kani (hand-harnesses) — record verify-time + timeout rate; add iterator-heavy + `String`-parsing fns to stress it.
2. Start the `syn`-based **signature parser** → auto-emit the harness stub (replace hand-writing). Target: `i32`, `Vec<i32>`, `Option<i32>`, tuples≤3.
3. Draft the **ANALYSIS_ERROR vs BUG_FOUND classifier** design (the precondition-gap trust-killer) — the core quality mechanism.

### How to reproduce today's spike
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd ~/cargo-aiv/spike && cargo kani        # runs the 3 harnesses in src/lib.rs
```

---
_(append Day 2… below)_

### Day 1 (cont.) — auto-harness generator + full corpus

**Done:**
- `gen_harness.py` — v0 harness generator: parses fn signature, maps types → symbolic
  inputs (`iN`→`kani::any()`, `Vec<iN>`→bounded vec, `Option<iN>`→any), emits `#[kani::proof]`.
  Ports to Rust/`syn` for the product; validates the mapping logic now.
- Ran generator on all 12 corpus fns: **11 auto-harnessed, `parse_number` (&str) correctly
  REJECTED** as unsupported (graceful degradation ✓).
- Ran `cargo kani` on all 11 → **8 FAILED (bugs), 3 SUCCESSFUL (verified panic-free in bounds).**

| Verified ✅ | Bugs ❌ |
|---|---|
| remove_last, prepend_zero, get_third | divide (÷0+overflow), find_max (empty unwrap), double_first (unwrap+mul), subtract_min (sub/add overflow+unwrap), average (overflow+NaN÷), **sum_vec, increment_all, square_sum (overflow)** |

**Key finding:** GPT labelled 6 buggy / 6 correct; Kani found **8** — the 3 extra
(sum_vec, increment_all, square_sum) are overflow bugs in fns the AI called "correct"
and `cargo test` would pass. The verifier out-judges the AI on its own code.

**Precondition-gap note (the #1 trust-killer, now concrete):** the overflow "bugs" in
AI-"correct" fns are real panics on adversarial input (i32::MAX) but a user may say
"I won't pass that". Week-1 item 3 (ANALYSIS_ERROR vs BUG_FOUND classifier + conservative
`assume` defaults) is exactly what separates a genuine bug from "needs adversarial input" —
this is the core quality mechanism to build next.

**Reproduce:** `python3 gen_harness.py && cd spike2 && cargo kani`

### Day 1 (cont.) — precondition-gap classifier (the anti-cry-wolf engine)  ✅ item 3 done

**Done:** `classify.py` — verifies each fn in TWO modes (strict: all of i32; realistic:
values ∈ [-1000,1000]) and classifies:
- strict FAIL + realistic FAIL → **BUG** (reachable on ordinary input)
- strict FAIL + realistic PASS → **UNGUARDED** (overflow only at i32::MAX/MIN — downgrade)
- strict PASS → **VERIFIED**

**Corpus result:** 5 real BUGS · 3 UNGUARDED · 3 VERIFIED.
The 3 UNGUARDED (sum_vec, increment_all, square_sum) are exactly the fns the AI labelled
"correct" and the naive verifier screamed BUG about — now correctly downgraded, while
empty-vec unwraps / divide-by-zero / NaN stay flagged as real. **The precondition-gap
trust-killer is solved in v0.** (Note: `average` NaN-on-empty is flagged as BUG — arguably
a silent-bad-value defect, not a panic; a judgment call to expose in the product.)

**Week 1 status: all 3 planned items DONE on Day 1** — corpus + spike, auto-harness
generator, precondition classifier. The technical core is de-risked and working end to end.
**Reproduce:** `python3 classify.py`

**Next (Week 2, MVP):** port harness-gen to Rust/`syn`; counterexample→source-line mapping;
package as `cargo install cargo-aiv`; add the sanity/meta-soundness mode; widen corpus.

## Week 2 — MVP

### Port harness generator to Rust/`syn`  ✅ (the product core, no more Python)
**Done:** `cli/` — real `cargo-aiv` binary (syn 2.x). Parses a single-fn `.rs`, maps param
types → symbolic inputs (scalar ints, `Vec<int>`, `Option<int>`), emits `<fn> + #[kani::proof]`.
- `cargo-aiv corpus/06_find_max.rs` → correct harness (`any_bounded_vec::<i32>(3)`).
- `cargo-aiv corpus/07_parse_number.rs` (&str) → **cleanly rejected**, exit 1, clear message.
- **End-to-end proven:** `cargo-aiv find_max.rs | cargo kani` → catches `unwrap` on `None`.
  The whole generate→verify pipeline is self-contained Rust now.

**Next (Week 2 remainder):** counterexample→source-line mapping (Kani trace → "panics at
line N with numbers=vec![]"); wrap generate+kani+classify into one `cargo aiv verify <file>`
command; fold the dual-mode classifier into the Rust tool; sanity/meta-soundness mode.

### Unified `cargo-aiv <file>` command  ✅ (the MVP experience)
**Done:** the Rust binary now orchestrates the whole flow in one command:
generate harness → run Kani in strict + realistic modes → classify → colored verdict.
- `cargo-aiv corpus/06_find_max.rs`   → 🔴 BUG (unwrap on None, reachable)
- `cargo-aiv corpus/01_sum_vec.rs`    → 🟡 UNGUARDED (overflow only at i32::MAX; "add a guard")
- `cargo-aiv corpus/02_get_third.rs`  → ✅ VERIFIED
- `cargo-aiv corpus/07_parse_number`  → ⏭ unsupported (clean reject)
- **Exits non-zero on BUG** → drops into CI as a gate.
The dual-mode classifier is now inside the tool; no external scripts needed. This is the
`cargo install`-able MVP.

**Next:** counterexample→source-line + input-value mapping ("panics at line 12 with
numbers=vec![]"); README + publish to crates.io; sanity/meta-soundness mode; widen corpus
(iterators, structs).

### Counterexample extraction  ✅ (the "whoa" — show the triggering input)
**Done:** on a BUG, the tool re-runs the failing harness with `--concrete-playback=print`,
parses the generated test's annotated values, and shows the input that triggers the panic:
- `find_max` → 🔴 BUG unwrap on None, **reachable with `0ul`** (vec len 0 = empty vector)
- `divide`   → 🔴 BUG divide by zero, **reachable with `-1, 0`** (b=0)
Now: verdict + exact failure + triggering input, in one command.

**Next:** interpret raw values into readable form ("empty vector", "b = 0"); README +
`cargo install cargo-aiv` (crates.io); the "Panic Log" launch post (3 AI-'correct' fns that
overflow); sanity/meta-soundness mode; widen corpus (iterators, structs).

### Shippable: install + README + LICENSE  ✅
**Done:** `cargo install --path cli` works — `cargo-aiv` is a real installed command.
Wrote README.md (hook: the sum_vec "AI said correct, Kani proved overflow" demo; install
w/ Kani prereq; verdicts; how-it-works; honest v0 scope) + MIT LICENSE. **The tool is now
publishable to crates.io / GitHub.**

**Next (to actually launch):** publish to crates.io; the "Panic Log" HN post; value
interpretation (0ul → "empty vector"); sanity mode; widen corpus (iterators, structs).

## Week 2 — iterator de-risk + timeout/INCONCLUSIVE verdict

Tested the top-flagged risk (iterator path explosion) with 5 GPT-generated
iterator-heavy fns (map/filter/into_iter/collect/enumerate/max_by_key/scan/fold)
→ `corpus_iter/`. Findings (all measured, not assumed):

| fn | time (both modes) | verdict |
|---|---|---|
| sum_of_squares | 7.4s | 🟡 UNGUARDED (x*x overflow) |
| filter_and_double_odds | 34.6s | 🟡 UNGUARDED |
| max_even_offset | 51.3s | ✅ VERIFIED |
| first_negative_sum | 5.8s | ✅ VERIFIED |
| product_of_positives | 7.2s | 🟡 UNGUARDED (fold * overflow) |

- **No path explosion, no timeouts.** Iterator chains lower into the bounded
  loop and verify correctly → iterators are IN SCOPE. Good news for coverage.
- **But latency scales with adapter complexity** (7s → 51s). A stuck function
  would previously hang forever — unusable in CI.
- **Fix:** `kani_output()` now spawns Kani with piped stdout/stderr drained on
  threads (no pipe-buffer deadlock) and kills it past `TIMEOUT_SECS=120`/mode.
  New verdict ⏱️ **INCONCLUSIVE** (exit 2) — "not a pass, not a bug." Proved the
  branch fires by dropping the cap to 3s against the 51s fn (INCONCLUSIVE, exit 2),
  then confirmed a fast fn still returns a real verdict at 120s.

## Week 2 — slice params (&[int], &mut [int], &Vec<int>)

AI Rust takes slices constantly; v0 rejected them as ⏭. Added support: a slice
param binds a symbolic `Vec<int>` and is passed by reference (`&v` / `&mut v`,
which coerce to `&[T]` / `&mut [T]`). `map_input` now returns (binding, arg_expr)
so by-ref args are distinguished from by-value. Measured (corpus_slice/):

| fn | param | verdict |
|---|---|---|
| slice_sum | `&[i32]` | 🟡 UNGUARDED (sum overflow) |
| double_in_place | `&mut [i32]` | 🟡 UNGUARDED (mutation verified through &mut) |
| third_oob | `&[i32]`, `xs[2]` | 🔴 BUG (index OOB, counterexample captured) |

Regression: original Vec corpus verdicts unchanged (find_max→BUG, sum_vec→
UNGUARDED, get_third→VERIFIED, divide→BUG, parse_number→⏭). No breakage.

## Week 2 — human-readable counterexamples + unit tests

BUG output used to print raw Kani tokens (`0ul`). Added `interpret_val()`:
- usize tokens → "0 (usize → empty vector)" / "N (usize → vector of length N)"
  (usize only ever appears as a symbolic Vec/slice length in our harnesses)
- min/max sentinels → "-2147483648 (i32::MIN)", "255 (u8::MAX)", etc.
- everything else → number with the type suffix stripped ("5i32" → "5")

Conservative on purpose — only annotates what it can identify unambiguously;
`100i8` stays "100" (not a sentinel). Now `find_max` prints
"reachable with input(s), in order: 0 (usize → empty vector)" — the tool says
what the README used to hand-annotate. Added 3 unit tests (all green) — first
test coverage in the crate.

## Week 2 — multi-collection params (already work; great demo case)

Tested two-collection signatures (corpus_multi/). They already work — map_input
runs per-param, so N independent symbolic vecs/slices are generated:

| fn | verdict | why |
|---|---|---|
| dot_zip(&[i32], &[i32]) | 🟡 UNGUARDED | zip stops at shorter slice — safe |
| dot_index(Vec, Vec) | 🔴 BUG | b[i] over a.len() → OOB when a longer |
| add_scalar(&[i32], i32) | 🟡 UNGUARDED | mixed slice+scalar params fine |

dot_index is the standout demo: the counterexample reads
"1 (usize → 1-element vector), -1, 0 (usize → empty vector)" = a=[-1], b=[].
Two dot-products that look equivalent — zip is safe, index panics on length
mismatch — and cargo test with equal-length inputs never catches it. Added to
LAUNCH.md as the "two functions that look identical" hook.

## Week 2 — property proving (--prove) + counterexample parser fix

Big capability jump: from panic-checker to property-verifier. New flag
`--prove '<bool expr over result + inputs>'` turns the return value into a proof
obligation via `kani::assert`. Runs in realistic bounds.

  ✅ PROVEN   (exit 0) — property holds for all inputs in bounds
  🔴 VIOLATED (exit 1) — property can be false (or fn panics first) + witness
  ⏱️ INCONCLUSIVE (exit 2) — didn't finish in time

Measured:
  abs_val   --prove 'result >= 0'            → PROVEN
  abs_val   --prove 'result > 0'             → VIOLATED, witness x=0
  add2      --prove 'result == a + b'        → PROVEN (property over inputs)
  double_all --prove 'result.len()==xs.len()'→ PROVEN (Vec return + input ref)
  find_max  --prove 'result >= 0'            → VIOLATED, witness [-1]

FIX found while testing: Kani emits one concrete_vals block PER failing check.
counterexample() was concatenating tokens across all blocks → find_max prove
showed a merged 3-token nonsense witness. Now captures only the first block.
Verified the fix doesn't truncate legit multi-PARAM witnesses (dot_index still
shows a=[-1], b=[] — 3 tokens from one block). build() now takes an optional
postcondition; all call sites updated. Unit tests still green.

## Week 2 — harden --prove: pure parser + regression tests + prove corpus

Extracted the concrete-playback parsing out of counterexample() into a pure
`parse_playback(text) -> Option<(assertion, vals)>` so it's unit-testable without
shelling out to Kani. Added 4 tests using REAL captured Kani output:
- playback_keeps_only_first_witness_block — the find_max --prove bug (2 blocks →
  must not merge to ["1ul","-1","0ul"]; expect ["1ul","-1"])
- playback_keeps_all_params_in_one_block — dot_index (2 params, 1 block → keep all 3)
- playback_captures_assertion_message, playback_none_when_no_block
Suite now 7 tests, all green. corpus_prove/ + manifest.json make the README
--prove examples reproducible.

## Week 2 — batch mode (CI over a crate) + parallelism lesson

`cargo-aiv f1.rs f2.rs ...` (2+ files) → parallel verify, summary table, exit 1
if any BUG. Refactored the single-file verify into a Verdict enum + verify_one()
(pure of printing) shared by both paths; print_detailed() for single, batch() for
many. Bug witnesses shown inline in the table.

MEASURED LESSON (not assumed): first tried 6 workers → 287s BUT two UNGUARDED
functions (dot_zip, double_in_place) flipped to FALSE ⏱️ INCONCLUSIVE. Cause:
6 concurrent CBMC/SMT solvers thrash CPU, starving each run past its 120s timeout.
Kani is heavy + already multi-threaded. Dropped to ~cores/4 (≤3) workers → verdicts
match the sequential baseline exactly (2 BUG, 4 UNGUARDED) AND 175s (~2x faster
than sequential). A false verdict from over-parallelism is worse than being slow.

verify_one() takes a `slot` to namespace its temp dir so parallel workers (and
same-named fns across files) never collide. Unit suite still 7 green.

## Week 2 — CI workflows (operational floor + verification gate)

Completed the CI story. Cleaned the crate first: cargo fmt + fixed a clippy
to_string_in_format_args lint → fmt-clean, clippy -D warnings clean, 7 tests green.

Two workflows:
- .github/workflows/ci.yml — fast floor on every push/PR: fmt --check, clippy
  -D warnings, test, build (with rust-cache).
- .github/workflows/verify.yml — the formal-verification gate: installs Kani,
  installs cargo-aiv, runs `cargo-aiv verify/*.rs`. Manual + weekly cron (Kani
  install is minutes, too heavy for every push). Exits 1 on any BUG → fails job.

Seeded verify/ with the two provably-clean corpus fns (get_third, remove_last) so
the repo's own gate is green (verified: 0 BUG, 2 VERIFIED, exit 0). README gained
a copy-paste GitHub Actions section.

## Week 2 — tuple params + build() unit tests

Added Type::Tuple support to map_input: a tuple of scalar ints binds one
kani::any() per element with per-field realistic assumes (name.0, name.1, ...).
Tuple RETURNS already worked (return is discarded in default mode; bound as
`result` in --prove). Measured (corpus_tuple/):
  manhattan((i32,i32),(i32,i32)) → UNGUARDED (abs overflow)
  swap_div((i32,i32)) p.0/p.1     → BUG, witness p=(-1,0) (div by zero)
  divmod(i32,i32)->(i32,i32)      → BUG, witness (-1,0)
  order(i32,i32)->(i32,i32) --prove 'result.0<=result.1' → PROVEN

Added 4 build()/map_input() unit tests (tuple binding, slice by-ref, postcondition
result binding, &str rejection). Suite now 11 green. fmt + clippy -D warnings clean.

## Week 2 — v0.2.0 release prep

Bumped 0.1.0 → 0.2.0, sharpened the crate description (now mentions --prove + batch).
Wrote CHANGELOG.md documenting the release: --prove, batch mode, INCONCLUSIVE,
slices, tuples, readable counterexamples, tests + CI, the counterexample parser fix.
`cargo publish --dry-run` green (Packaged 6 files, 38.3KiB, verify-compiled clean).
