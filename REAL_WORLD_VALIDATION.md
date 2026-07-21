# Real-world validation — what cargo-vouch does on code it didn't choose

The dogfood in [`RESULTS.md`](RESULTS.md) is honest but *small and self-authored*.
This is the unbiased test: two real crates pulled from crates.io — **not** written to
suit the tool — run at default settings, every verdict reported.

Reproduce:

```bash
cargo new probe && cd probe
cargo add roman levenshtein && cargo fetch
# strip only the crate-level `#![doc = include_str!(...)]` / test modules (they don't
# compile out of the crate); the function bodies are verbatim.
cargo-vouch roman_api.rs      # the 5 library fns
cargo-vouch lev.rs            # levenshtein(a, b)
```

## Result: 6 / 6 functions → ⏱️ INCONCLUSIVE

| Crate | Function | Param types | Verdict |
|---|---|---|---|
| `roman-0.2.1` | `to(n: i32)` | scalar | ⏱️ INCONCLUSIVE |
| `roman-0.2.1` | `to_lower(n: i32)` | scalar | ⏱️ INCONCLUSIVE |
| `roman-0.2.1` | `from(txt: &str)` | string | ⏱️ INCONCLUSIVE |
| `roman-0.2.1` | `from_lower(txt: &str)` | string | ⏱️ INCONCLUSIVE |
| `roman-0.2.1` | `from_lax(txt: &str)` | string | ⏱️ INCONCLUSIVE |
| `levenshtein-1.0.5` | `levenshtein(a: &str, b: &str)` | string, string | ⏱️ INCONCLUSIVE |

**Zero verdicts.** Not one BUG, UNGUARDED, or VERIFIED on real code.

## Why — and why it is not a false verdict

The tool did the right thing: it returned INCONCLUSIVE, never a false ✅. The cause is
the same in every case — **real functions iterate**, and bounded model checking unwinds
loops only to a fixed shallow depth:

- `roman::to` runs `while n >= value { n -= value; out.push_str(name) }` — up to ~3999
  iterations for a valid input. No practical unwind bound reaches that.
- `roman::from_lax` iterates `txt.chars().rev()`, calls `to_ascii_uppercase()` (Unicode),
  and scans the 7-element `ROMAN` table with `.iter().find()` — three nested sources of
  unwinding. Even at `--str-unwind 8` it timed out (4 min) rather than complete.
- `roman::from` / `from_lower` call `to`, inheriting its unbounded loop.
- `levenshtein` builds `(1..).take(n).collect()` and runs nested char-iteration over a
  `Vec` cache — the classic O(n·m) loop nest.

Raising `--unwind` / `--str-unwind` doesn't rescue these: it just trades
INCONCLUSIVE-by-instability for INCONCLUSIVE-by-timeout.

## What this means for the tool's scope (honestly)

cargo-vouch's demonstrated wins ([`RESULTS.md`](RESULTS.md)) are all **loop-light**
functions — the bug lives in the *arithmetic, indexing, or an `unwrap`*, not behind deep
iteration: divide-by-zero page math, empty-slice `unwrap`, `i32::MIN` overflow,
`Option::unwrap()` on `None`, `parse().unwrap()` on empty input. That is a real and
common bug class — a lot of AI-generated helper code is exactly this shape — and the tool
catches it with a concrete witness, which a test suite does not.

But it is **not** a general "point it at any Rust function and get a verdict" tool. On
iteration-heavy real code — parsers, string algorithms, anything with a data-dependent
loop — expect ⏱️ INCONCLUSIVE. The honest headline is:

> **cargo-vouch proves panic-freedom of loop-light functions. On functions dominated by
> data-dependent or nested iteration, it reports INCONCLUSIVE — never a false pass, but
> also no answer.**

This is a reach limit of bounded model checking, not a defect in the classifier.
