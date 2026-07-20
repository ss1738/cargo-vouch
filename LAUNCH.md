# The Panic Log: I asked an AI for 12 Rust functions, then *proved* how many were actually correct

*(draft launch post — HN / r/rust / blog. All numbers are real, reproducible from the repo.)*

---

AI writes a lot of our code now. We check it with `cargo test` — which only tests the cases
we thought of. So I ran an experiment: I asked GPT-4o for 12 small Rust utility functions,
**and asked it to label each one `buggy` or `correct` itself.** Then I formally verified all
of them with bounded model checking (Kani) — not testing a sample, *proving* the property for
every input in bounds.

The AI got its own code wrong **4 out of 11 times.**

## The results

| Function | AI said | Formal verdict | |
|---|---|---|---|
| `sum_vec` | correct | 🟡 overflows | **AI wrong** |
| `increment_all` | correct | 🟡 overflows | **AI wrong** |
| `square_sum` | correct | 🟡 overflows | **AI wrong** |
| `prepend_zero` | buggy | ✅ verified safe | **AI wrong** (false alarm) |
| `find_max` | buggy | 🔴 panics (empty vec) | ✓ |
| `divide` | buggy | 🔴 divide-by-zero | ✓ |
| `double_first` | buggy | 🔴 panics (empty vec) | ✓ |
| `subtract_min` | buggy | 🔴 panics (empty vec) | ✓ |
| `average` | buggy | 🔴 NaN on empty | ✓ |
| `get_third` | correct | ✅ verified safe | ✓ |
| `remove_last` | correct | ✅ verified safe | ✓ |

(A 12th, `parse_number(s: &str)`, was outside the tool's scope and skipped — not answered wrongly.)

## Three things this shows

**1. The AI called broken code "correct" — three times.** `sum_vec` is just
`numbers.iter().sum()`. It passes `cargo test`. GPT said it was correct. But `[i32::MAX, 1]`
overflows it. Same for `increment_all` and `square_sum`. **A test suite would never catch
this. A proof does, instantly.**

**2. The AI called *correct* code "buggy" too.** `prepend_zero` is fine — the AI's own
risk-assessment was a false alarm. So you can't trust the model to grade the model; you need
an oracle that isn't another LLM.

**3. Not all "bugs" are equal — and a good tool says so.** Those three overflows only happen
at `i32::MAX/MIN`. That's real, but adversarial — you'd never pass `i32::MAX` to a sum. So the
tool **downgrades them to 🟡 UNGUARDED ("add a guard"), not 🔴 BUG.** The genuine 🔴 bugs — an
empty vector hitting `.unwrap()`, a zero divisor — get flagged as reachable on ordinary input,
*with the exact triggering value.* The difference between crying wolf and being trusted is
this distinction, and it's the whole game.

## Try it

```bash
cargo install --locked kani-verifier && cargo-kani setup   # the verification engine
cargo install cargo-aiv
cargo-aiv your_function.rs
```

```console
$ cargo-aiv find_max.rs
🔴 BUG  `find_max` — panic reachable on ordinary input:
     • called `Option::unwrap()` on a `None` value
     reachable with input(s), in order: 0 (usize → empty vector)
```

Zero annotations. Paste the function, get a verdict. It exits non-zero on a real bug, so it
drops into CI.

## Bonus: two functions that look identical, one panics

After the corpus run I threw two dot-products at it — the kind of thing an AI emits
without thinking about length:

```rust
fn dot_zip(a: &[i32], b: &[i32]) -> i32 {           // 🟡 UNGUARDED (overflow only)
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}
fn dot_index(a: Vec<i32>, b: Vec<i32>) -> i32 {     // 🔴 BUG
    let mut s = 0;
    for i in 0..a.len() { s += a[i] * b[i]; }        // b[i] panics if a is longer
    s
}
```

`dot_zip` is safe — `zip` stops at the shorter slice. `dot_index` panics the moment
the lengths differ, and `cargo-aiv` prints the exact witness:

```console
🔴 BUG  `dot_index` — index out of bounds: the length is less than or equal to the given index
     reachable with input(s), in order: 1 (usize → 1-element vector), -1, 0 (usize → empty vector)
```

`a = [-1], b = []`. Every hand-written test I'd write passes them the same length and
sees nothing. The proof doesn't.

## Aside: I pointed it at itself

The scary failure mode for a verifier isn't missing a bug — it's *claiming a bug that
isn't there*, or worse, silently passing everything. So I generated a fresh batch of AI
functions it had never seen (slices, tuples, `Option`) and read every surprising verdict.
It caught **two false verdicts in its own classifier**:

- `factorial(n) = (1..=n).product()` was flagged 🔴 BUG — but the failure was Kani's
  *unwinding assertion* (the loop needs ~1000 iterations, more than the unwind bound), not
  a panic. Fixed: that's now ⏱️ INCONCLUSIVE ("raise `--unwind`"), and at `--unwind 15` it
  finds the *real* bug — `factorial(13)` overflows i32.
- `Option<i32>` payloads weren't range-clamped in "realistic" mode, so
  `opt.map(|x| x + 1)` overflowing at `Some(i32::MAX)` read as 🔴 BUG instead of 🟡
  UNGUARDED. Fixed — and verified the clamp still lets a genuine `.unwrap()`-on-`None`
  surface as a real BUG.

A verifier you can't trust is worse than none. `cargo-aiv --selftest` runs a known-bug
and known-safe function and refuses to vouch for its results if it can't tell them apart.

## The point

Tests are probabilistic; proofs aren't. As more code comes from models that are confidently
wrong about their own edge cases, "the tests pass" stops being enough. **Prove, don't pray.**

`cargo-aiv` is MIT-licensed and open source. It's v0 — narrow on purpose (safe Rust,
panic-freedom + overflow, bounded inputs; scalars, `Vec`/slices/tuples/`Option` of ints).
It stands on [Kani](https://github.com/model-checking/kani); the new part is the
zero-annotation, AI-aware harness generator, the BUG/UNGUARDED/INCONCLUSIVE classifier,
`--prove` for postconditions, and parallel batch mode for CI.

*Repo: github.com/ss1738/cargo-aiv · reproduce every number above with the corpora in
`/corpus*`.*
