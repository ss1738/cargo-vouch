# cargo-aiv

**Prove AI-generated Rust is panic-free — don't just test it.**

AI writes an exploding share of your code, and `cargo test` only checks the cases you
thought of. `cargo-aiv` auto-generates a formal-verification harness for **every function
in a file**, runs bounded model checking (via [Kani](https://github.com/model-checking/kani)),
and tells you whether each can *panic or overflow* — for **all** inputs in bounds, not a sample.

```console
$ cargo-aiv sum_vec.rs
🟡 UNGUARDED  `sum_vec` — safe on normal input, but overflows at i32::MAX/MIN:
     • attempt to add with overflow
     add a bounds guard or use checked_/saturating_ arithmetic.
```

That function was written by an AI that called it *correct*, and `cargo test` passes. Kani
**proves** it overflows on `[i32::MAX, 1]`. That's the difference between a test and a proof.

It also **proves postconditions** (`--prove 'result >= 0'`), **gates a whole directory** in
parallel (`cargo-aiv src/`, exit 1 on any bug), speaks `--json` for CI, and
`--selftest`s itself so it never silently vouches for results it can't actually check.

Run on 14 functions of ordinary utility code, it found **five reachable panics `cargo test`
would ship** (divide-by-zeros, empty-slice unwraps) — see [`RESULTS.md`](RESULTS.md).

## Why

- **Tests are probabilistic. Proofs aren't.** Formal methods check every input in bounds.
- **AI code has a blind spot for edge cases** — empty vectors, integer overflow, `unwrap()`
  on `None`, divide-by-zero. `cargo-aiv` finds them before you ship.
- **Zero annotations.** You don't write specs or learn a verification language. Paste the
  function, get a verdict.

## Verdicts

| | Meaning |
|---|---|
| 🔴 **BUG** | Panic reachable on *ordinary* input — with the exact triggering value. Fix it. |
| 🟡 **UNGUARDED** | Only overflows at `i32::MAX/MIN`. Real, but adversarial — add a guard. *Not a false alarm.* |
| ✅ **VERIFIED** | Provably panic-free within bounds (Vec ≤ 3, values ≤ 1000). |
| ⏱️ **INCONCLUSIVE** | Verification didn't finish in 120s/mode (too complex at the current bounds). *Not a pass, not a bug* — exits `2`. |
| ⏭ **unsupported** | Uses a type outside v0 scope — skipped cleanly, never a wrong answer. |

`cargo-aiv` **exits non-zero on a BUG**, so it drops straight into CI as a gate.

```console
$ cargo-aiv find_max.rs
🔴 BUG  `find_max` — panic reachable on ordinary input:
     • called `Option::unwrap()` on a `None` value
     reachable with input(s), in order: 0 (usize → empty vector)
```

## Batch mode (CI over a whole crate)

Pass more than one file — or a **directory** (recursed for `.rs`) — and `cargo-aiv`
verifies every function in parallel, prints a summary table, and exits `1` if **any**
function has a BUG. `cargo-aiv src/` gates a whole crate in one command:

```console
$ cargo-aiv src/
cargo-aiv batch — 6 file(s), 3 workers, ≤120s/mode each

  dot_zip                🟡 UNGUARDED
  dot_index              🔴 BUG  ← 1 (usize → 1-element vector), -1, 0 (usize → empty vector)
  add_scalar             🟡 UNGUARDED
  slice_sum              🟡 UNGUARDED
  third                  🔴 BUG  ← 0 (usize → empty vector)
────
2 BUG · 3 UNGUARDED · 1 VERIFIED · 0 other
```

**Choose your gate's strictness** with `--fail-on`: `bug` (default — only reachable
panics fail), `unguarded` (also fail on extreme-input overflows), or `inconclusive`
(also fail anything that couldn't be proven). Exit 1 when the level is tripped.

Concurrency is deliberately capped low (~cores/4, ≤3): Kani/CBMC is heavy and
already multi-threaded, so running too many at once starves each solver past its
timeout and yields false INCONCLUSIVE. Low-but-parallel is both correct and ~2×
faster than sequential.

### GitHub Actions

Put the functions you want proven panic-free in a `verify/` directory and gate on
them (this repo ships a working copy in `.github/workflows/verify.yml`):

```yaml
- name: Install Kani
  run: cargo install --locked kani-verifier && cargo kani setup
- name: Install cargo-aiv
  run: cargo install cargo-aiv
- name: Prove verify/ is panic-free   # exits 1 on any BUG → fails the job
  run: cargo-aiv verify/*.rs
```

Because a Kani install is minutes, run this on a schedule / `workflow_dispatch`
rather than every push — keep `cargo test` + `clippy` on the hot path (see
`.github/workflows/ci.yml`).

### Machine-readable output

Add `--json` (single file or batch) for structured results you can post to a PR or
gate on programmatically:

```console
$ cargo-aiv --json src/*.rs | jq '.summary'
{ "bug": 1, "unguarded": 0, "verified": 1, "other": 0 }
```

Output is `{"results": [{"name", "verdict", "checks", "witness"}, …], "summary": {…}}`
— one entry per function (a file with many functions yields many results). Exit code is
unchanged (1 if any BUG).

## Prove properties, not just panic-freedom

Panic-freedom is the default. To prove a **postcondition** about the return value, pass
`--prove` with a boolean Rust expression over `result` (and the input names):

```console
$ cargo-aiv --prove 'result >= 0' abs_val.rs
✅ PROVEN  `abs_val` — result >= 0 holds for all inputs in bounds.

$ cargo-aiv --prove 'result.len() == xs.len()' double_all.rs
✅ PROVEN  `double_all` — result.len() == xs.len() holds for all inputs in bounds.

$ cargo-aiv --prove 'result > 0' abs_val.rs
🔴 VIOLATED  `abs_val` — result > 0 can be false (or the fn panics first):
     • aiv postcondition
     counterexample input(s), in order: 0
```

`PROVEN` exits `0`, `VIOLATED` exits `1` — so a postcondition is a CI gate too. The
property is checked for all inputs within the realistic bounds (values ≤ 1000, Vec ≤ 3).

## Install

Requires [Kani](https://model-checking.github.io/kani/install-guide.html) (the verification engine):

```bash
cargo install --locked kani-verifier
cargo-kani setup
```

Then:

```bash
cargo install cargo-aiv        # (or: cargo install --path cli)
cargo-aiv path/to/function.rs
```

## How it works

1. **Parse** the function signature with `syn`.
2. **Synthesise** a Kani proof harness — each parameter becomes a symbolic input
   (`i32 → kani::any()`, `Vec<i32> →` a bounded symbolic vec, `Option<i32> → kani::any()`).
3. **Verify twice** — *strict* (all of `i32`) and *realistic* (values ≤ 1000). A panic that
   survives realistic bounds is a real **BUG**; one that only appears at `i32::MAX` is
   **UNGUARDED**. This dual-mode check is what stops it crying wolf on `a + b`.
4. **Report** the verdict + the concrete counterexample.

## Scope (v0 — honest about the boundaries)

**Supported:** safe Rust, every free function in a file (methods/`self` skipped),
parameters of scalar ints (`i8..u64`, `bool`),
`Vec<int>`, `Option<int>`, int **slices** (`&[int]`, `&mut [int]`, `&Vec<int>` — mutation
through `&mut` is verified too), **`&str`/`String`** (bound as a symbolic ASCII string,
length ≤ bound), and **tuples of scalar ints** (`(i32, i32)`, …) as params;
tuple returns work in both default and `--prove` mode (`result.0`, `result.1`). Property:
**panic-freedom + integer overflow**, bounded (Vec ≤ 3, loops unwound ≤ 5). **Idiomatic iterator chains verify fine** — `.iter().map().filter()
.collect()`, `.fold()`, `.scan()`, `.enumerate()`, `.max_by_key()` all lower into the bounded
model; they don't path-explode, though heavy adapter chains can take ~30–50s. Past 120s/mode
the tool reports ⏱️ INCONCLUSIVE instead of hanging.

**On strings, honestly:** `&str`/`String` catch the classic string panics — `.chars().next()
.unwrap()`, `.parse().unwrap()` on empty or malformed input — with a witness, at default
settings (~30–40s). Strings are ~10× more expensive under bounded model checking than
int/`Vec` params (UTF-8 decode + Unicode machinery), and the cost is driven by the *unwind
depth*, not the length — so string harnesses get their **own low defaults, decoupled from the
numeric knobs**: length ≤ 1 (`--str-bound`, default 1) and unwind 2 (`--str-unwind`,
default 2). That's what makes a string bug resolve to 🔴/✅ instead of ⏱️ INCONCLUSIVE out of
the box. Length ≤ 1 still covers the empty-string case, where nearly every string panic lives.
Raise `--str-bound`/`--str-unwind` for more coverage at higher cost. Heavy text transforms
(`.to_uppercase()`, multi-`.split()`) can still time out to ⏱️ INCONCLUSIVE — never a false ✅.
*Known cosmetic gap:* a string witness is printed in the `Vec` idiom (e.g. "1-element
vector") — the reproducing input is right, the noun is not yet string-aware.

**Tuning the rigor:** `--bound N` sets the max Vec/slice length checked (default 3),
`--unwind N` the loop-unroll depth (default 5). Higher = more coverage, slower. A
✅ VERIFIED is only a proof *within* these bounds — raise them for stronger guarantees.

**Not yet:** functional correctness ("does it sort?"), `unsafe`, generics/traits, floats,
recursion, unbounded loops, external crates, `&str`/`String`, custom types. These are
rejected cleanly (⏭), never answered wrongly.

## License

MIT.
