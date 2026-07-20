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
     reachable with symbolic input(s): 0ul      # the empty vector
```

Zero annotations. Paste the function, get a verdict. It exits non-zero on a real bug, so it
drops into CI.

## The point

Tests are probabilistic; proofs aren't. As more code comes from models that are confidently
wrong about their own edge cases, "the tests pass" stops being enough. **Prove, don't pray.**

`cargo-aiv` is MIT-licensed and open source. It's v0 — narrow on purpose (safe Rust,
panic-freedom + overflow, bounded inputs; scalar/`Vec`/`Option` of ints). It stands on
[Kani](https://github.com/model-checking/kani); the new part is the zero-annotation,
AI-aware harness generator + the BUG/UNGUARDED classifier.

*Repo: github.com/ss1738/cargo-aiv · reproduce every number above with the corpus in `/corpus`.*
