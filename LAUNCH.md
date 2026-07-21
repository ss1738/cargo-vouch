# I asked GPT-4o for 12 Rust functions and checked how many were actually correct

*(draft launch post for Show HN / r/rust / LinkedIn. Numbers are real and reproducible from the repo. Rewrite in your own voice before posting.)*

I write a fair amount of Rust with an LLM these days. `cargo test` only checks the cases I remember to write, so I wanted something that checks a function against every input, not a sample. I built a small tool, cargo-vouch. It generates a Kani proof harness for each function in a file automatically and runs bounded model checking, then tells you whether the function can panic or overflow.

To try it out I asked GPT-4o for 12 small utility functions and told it to label each one buggy or correct. Then I ran cargo-vouch on all of them.

It got its own code wrong 4 times out of 11.

## The results

| Function | GPT-4o said | Verdict | |
|---|---|---|---|
| `sum_vec` | correct | overflows | GPT wrong |
| `increment_all` | correct | overflows | GPT wrong |
| `square_sum` | correct | overflows | GPT wrong |
| `prepend_zero` | buggy | verified safe | GPT wrong (false alarm) |
| `find_max` | buggy | panics (empty vec) | ok |
| `divide` | buggy | divide-by-zero | ok |
| `double_first` | buggy | panics (empty vec) | ok |
| `subtract_min` | buggy | panics (empty vec) | ok |
| `average` | buggy | NaN on empty | ok |
| `get_third` | correct | verified safe | ok |
| `remove_last` | correct | verified safe | ok |

(The 12th took a `&str` and was out of scope, so it was skipped rather than answered wrong.)

Three of the ones GPT called "correct" overflow. `sum_vec` is just `numbers.iter().sum()`. It passes `cargo test`. GPT said it was correct. It overflows on `[i32::MAX, 1]`. Same story for `increment_all` and `square_sum`. A test suite that passes normal inputs never hits that case.

It also called a correct function buggy. `prepend_zero` is fine; the model's own risk guess was just wrong. So grading the model with the model doesn't work. You need something that isn't another LLM.

One thing I care about: not every overflow is worth the same alarm. Those three only overflow at `i32::MAX/MIN`, which you would never actually pass to a sum. cargo-vouch marks those UNGUARDED (add a guard) instead of BUG, so it isn't screaming about adversarial inputs. The genuine bugs, an empty vector hitting `.unwrap()` or a zero divisor, get flagged as reachable on ordinary input, with the exact value that triggers them.

## I also ran it on ordinary utility code

To check it wasn't a trick that only works on cherry-picked functions, I wrote three plain modules of the kind most projects have (stats, pagination, geometry, 14 functions total) and ran the whole directory. It found five reachable panics that `cargo test` would have shipped:

```rust
fn page_count(total: i32, per_page: i32) -> i32 { (total + per_page - 1) / per_page }
//   divide-by-zero when per_page == 0
fn mean(xs: &[i32]) -> i32 { xs.iter().sum::<i32>() / xs.len() as i32 }
//   divide-by-zero on an empty slice
fn maximum(xs: &[i32]) -> i32 { *xs.iter().max().unwrap() }
//   .unwrap() on None for an empty slice
```

Every one is an empty-input or zero-divisor panic, which is the class of bug that survives a test suite because tests pass non-empty, sensible inputs. The overflow-only functions came back UNGUARDED, and the one function I wrote defensively verified as safe. Full run with timings is in RESULTS.md, and every verdict there was actually run under Kani, none inferred.

## Where it does not work (worth knowing before you install)

cargo-vouch is narrow on purpose. It works when the bug lives in the arithmetic, an index, or an `.unwrap()`. It does not work on functions with data-dependent loops, because bounded model checking runs out of unwinding depth. I pulled two random crates off crates.io (roman and levenshtein) and it returned INCONCLUSIVE on all 6 of their functions, because those functions parse and iterate. The README says this up front. Point it at loop-light code, not at your parser.

It is also a layer on top of [Kani](https://github.com/model-checking/kani), which does the actual verification. The part I built is the zero-annotation harness generation and the BUG/UNGUARDED/VERIFIED/INCONCLUSIVE classifier that will not report a pass it cannot back up. There is a `--selftest` that runs a known-bug and a known-safe function and refuses to trust its own results if it can't tell them apart.

## Try it

It isn't on crates.io yet, so install from source:

```bash
cargo install --locked kani-verifier && cargo-kani setup   # the verification engine
git clone https://github.com/ss1738/cargo-vouch && cargo install --path cargo-vouch/cli
cargo-vouch src/                                            # a file, or a whole directory
```

```console
$ cargo-vouch find_max.rs
🔴 BUG  `find_max` — panic reachable on ordinary input:
     • called `Option::unwrap()` on a `None` value
     reachable with input(s), in order: 0 (usize → empty vector)
```

No annotations. Paste the function, get a verdict. It exits non-zero on a real bug, so it works as a CI gate.

MIT licensed. Repo: github.com/ss1738/cargo-vouch. There is also an experimental agent in `agent/` that has an LLM write a function, checks it with cargo-vouch, and feeds the counterexample back until it verifies.
