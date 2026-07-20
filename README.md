# cargo-aiv

**Prove AI-generated Rust is panic-free — don't just test it.**

AI writes an exploding share of your code, and `cargo test` only checks the cases you
thought of. `cargo-aiv` auto-generates a formal-verification harness for a function, runs
bounded model checking (via [Kani](https://github.com/model-checking/kani)), and tells you
whether it can *panic or overflow* — for **all** inputs in bounds, not a sample.

```console
$ cargo-aiv sum_vec.rs
🟡 UNGUARDED  `sum_vec` — safe on normal input, but overflows at i32::MAX/MIN:
     • attempt to add with overflow
     add a bounds guard or use checked_/saturating_ arithmetic.
```

That function was written by an AI that called it *correct*, and `cargo test` passes. Kani
**proves** it overflows on `[i32::MAX, 1]`. That's the difference between a test and a proof.

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
     reachable with symbolic input(s), in order: 0ul   # empty vector
```

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

**Supported:** safe Rust, single function, parameters of scalar ints (`i8..u64`, `bool`),
`Vec<int>`, `Option<int>`. Property: **panic-freedom + integer overflow**, bounded (Vec ≤ 3,
loops unwound ≤ 5). **Idiomatic iterator chains verify fine** — `.iter().map().filter()
.collect()`, `.fold()`, `.scan()`, `.enumerate()`, `.max_by_key()` all lower into the bounded
model; they don't path-explode, though heavy adapter chains can take ~30–50s. Past 120s/mode
the tool reports ⏱️ INCONCLUSIVE instead of hanging.

**Not yet:** functional correctness ("does it sort?"), `unsafe`, generics/traits, floats,
recursion, unbounded loops, external crates, `&str`/`String`, custom types. These are
rejected cleanly (⏭), never answered wrongly.

## License

MIT.
