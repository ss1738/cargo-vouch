# cargo-aiv — Formal verification for AI-generated code

**Prove AI-generated code is panic-free, don't just test it.** A CLI + VS Code extension
that auto-synthesises a formal-verification harness for an AI-generated Rust function,
runs bounded model checking (Kani), and returns ✅ *"no panic for all bounded inputs"* or
❌ *"panics with `vec![-1, 0]`"*.

Synthesised from a 4-model plan (GPT-4o, Kimi, Qwen, Claude), 2026-07-20. Decisions and
rationale below are the founder's (single technical founder, formal-methods + ML, ships fast).

---

## 0. The one-line thesis

AI writes an exploding share of code; testing is probabilistic (checks the cases you thought
of); the hard, defensible answer is **formal methods** — *prove* properties hold for all
inputs. The moat is **difficulty**: almost no dev-tools founder can do formal methods; you've
shipped SMT/Z3 verification (FINGAURD). "Prove, don't pray."

**Proven day 1** (see WORKLOG): on real AI-generated Rust, the verifier caught a divide-by-zero,
an `i32::MIN/-1` overflow, an empty-vec `unwrap`, **and an integer overflow in a function the AI
labelled "correct" and `cargo test` would pass.**

---

## 1. Precise wedge (narrow — the scope guards ARE the product)

- **Product:** `cargo-aiv` — CLI + VS Code extension.
- **Language:** Rust (safe Rust only; **no** `unsafe`, `async`, generics beyond `Vec<T>`/`Option<T>`).
- **Property:** **panic-freedom + integer overflow** on **bounded inputs** (Vec len ≤ 3, loop unwind ≤ 5). NOT functional correctness ("does it sort?").
- **User:** solo devs / small teams using Cursor/Copilot to generate small Rust utilities, who want to catch edge-case crashes *before* running the code.
- **The moment:** dev pastes an AI-generated `fn`; `cargo-aiv` synthesises a Kani harness, runs BMC, returns ✅/❌ with a concrete counterexample.
- If it fails on `fn fib(n)` (recursion) — that's **by design, a boundary, not a bug.**

## 2. Technical approach

**Be the AI-aware UX layer on a mature engine — do NOT build a Rust→SMT translator (2-yr research).**
- **Backend:** **Kani** (AWS's open-source Rust bounded model checker; lowers safe Rust → GOTO-C → CBMC/SMT). You build the *harness synthesiser* + *result interpreter*.
- **Harness synthesis (the core IP):** parse the `fn` signature with `syn` →
  `i32 → kani::any()`, `Vec<i32> → bounded symbolic vec (len ≤ BOUND)`, `Option<i32> → kani::any()`.
- **"Spec inference" (2026-feasible):** lightweight keyword heuristics on docstring/prompt
  (`non-empty`, `positive`, `sorted` → `kani::assume!`). **NOT** semantic inference (research-hard).
- **Property:** panic-freedom by default; treat `assert!` in the body as post-conditions.

**Hardest problems (both models flagged #1 as make-or-break):**
1. **The precondition gap (#1 trust-killer):** AI code assumes implicit preconditions; test all inputs and Kani "finds a panic" that's just a missing guard. Assume too much → miss real bugs; too little → everything looks broken. *This is exactly your FINGAURD SMT edge.* Classify harness-assumption failures as `ANALYSIS ERROR`, not `BUG FOUND`.
2. **Harness gen for compound invariants** (sorted `Vec`, valid UTF-8 `String`, `NonZeroU32`).
3. **Iterator path explosion** (`.iter().map().collect()` → SMT blowup → timeouts).
4. **Counterexample → readable source** (CBMC trace → `vec![0, 5, -2]` + editor line).
5. **Meta-soundness:** a harness-gen bug that constrains inputs to empty → false `SUCCESS`. Needs a sanity mode (mutate code, confirm the harness catches it).

**Feasible in 2026:** panic-freedom for scalar arith, indexing, `unwrap`/`expect`, bounded loops, `Vec`/`Option`; verify times 0.4–60s. **Research-hard (DO NOT TOUCH v0):** loop-invariant inference, functional correctness, heap/`Rc<RefCell>`, generics/traits, `unsafe`, floats, recursion.

## 3. Roadmap (~12 weeks, solo)

| Phase | Weeks | Milestones | Kill criterion |
|---|---|---|---|
| **Spike / de-risk** | 1–3 | 20 AI Rust fns → Kani harnesses → measure verify-time + timeout rate; `syn` signature parser | **<60% verify in <60s → pivot backend/narrow scope** |
| **MVP** | 4–6 | Auto-harness (primitives, `Vec`, `Option`, tuples≤3), Kani orchestration, counterexample→source, `cargo install cargo-aiv` | — |
| **Quality** | 7–9 | Dogfood 100 AI fns → classify VERIFIED / REAL_PANIC / MISSING_PRECONDITION / TIMEOUT / UNSUPPORTED; **sanity mode**; graceful degradation | — |
| **Launch** | 10–12 | VS Code ext + GitHub Action + HN/r/rust/Lobsters + "The Panic Log" blog series | — |

> **v0 ships at Week 6.** Everything after improves trust + adoption.
> **Spike de-risked on Day 1** (WORKLOG): Kani found all 3 seeded/hidden bugs in <1s each. The kill criterion is already cleared for these shapes.

## 4. Quality bar ("good enough to ship")

- **Sound within bounds** — if it says ✅, it's true for the bounded model. **Always display the bound**: *"verified panic-free for Vecs ≤ 3, loops ≤ 5."*
- **Completeness not required** — `TIMEOUT`/`UNDECIDABLE` acceptable.
- **False bug reports: near-zero tolerance.** A shown panic must be real + reproducible >90%. Harness-assumption artifacts → `ANALYSIS ERROR`, never `BUG FOUND`. *Crying wolf destroys trust instantly.*
- **Perf:** result in ≤60s for ~50-LOC fns. **Graceful degradation:** unsupported construct → *"Unsupported: uses `unsafe` at line 5"* in <2s, never hang/crash.

## 5. Competition & wedge

| Competitor | What | Your angle |
|---|---|---|
| **Kani** | Rust BMC; **manual** harnesses | You are the **auto-harness Copilot** for Kani — the AI UX layer it lacks |
| **Certora** | Solidity, manual CVL spec, audit-firm | You: Rust, automated, zero-annotation |
| **Dafny / F* / Why3** | verification-first langs, heavy annotation | You verify **existing** Rust with **zero** annotations |
| **Copilot / Cursor** | generate code + tests/lints | tests are probabilistic; you do **exhaustive BMC** — "prove, don't pray" |
| **Clippy / Miri / Rudra** | static analysis / interpreter | you catch **deep semantic** edge cases via SMT (the `unwrap` that fails when the AI regex doesn't match) |

**Wedge:** own the *AI-generated-code → proof-of-panic-freedom* bridge. Kani exists; nobody built the one-click harness generator that makes it usable for AI-assisted workflows. You sell **panic insurance to AI users who don't know what SMT is.**

## 6. Go-to-market

- **Free OSS:** MIT/Apache-2, `cargo install cargo-aiv`, CLI-first (what Rust devs expect).
- **Content:** **"The Panic Log"** — generate 100 Rust fns with GPT/Claude, run `cargo-aiv`, document the edge-case panics `cargo test` missed. Highly shareable. (Day-1 spike is already exhibit A: an overflow the AI called "correct".)
- **Community:** Cursor/Windsurf Discords, Rust Zulip/r/rust. Position: safety net for AI-assisted coding.
- **Monetise (post-MVP):** cloud CI gate ($25/seat, higher bounds, parallel), custom property templates (Pro, $20 indiv / $100 team), enterprise air-gapped runner for fintech/health Rust shops.

## 7. Top risks + 30-day de-risk

| Risk | Kills you because | De-risk (first 30 days) |
|---|---|---|
| **Kani chokes on idiomatic AI Rust** (`.collect()`, iterators → timeout) | product looks broken | ✅ **DE-RISKED Day 1** — corpus fns verified in <1s. Keep measuring on iterator-heavy fns wk 1–2; if >40% timeout, narrow scope or add concolic fallback |
| **Harness gen too weak for real types** (custom struct/enum) | coverage too narrow | wk 2: run signature→harness on 100 crates.io fns; if >50% fail, scope to "scalar + Vec only", market as "verify your algorithms" |
| **Precondition gap → false bug reports** | #1 trust-killer | wk 1–3: build the ANALYSIS_ERROR classifier + conservative-assume defaults (your SMT edge) |
| **Devs don't value panic-freedom** | luxury not painkiller | wk 3: post "5 scary panics in AI Rust that passed `cargo test`" (you have the material). Tepid response → consider Solidity (panic = lost money) |
| **Kani install friction** (CBMC, toolchain) | casual users bounce | ship a Docker wrapper or remote cloud-check fallback; don't assume users tolerate build pain |
| **Scope creep into "real" verification** | burnout | constraints in stone: no unsafe / external crates / unbounded loops / generics-with-bounds. Everything else → POST-MVP.md |

---

**Bottom line:** you're not building a verifier — you're building the *harness-synthesising,
result-explaining, AI-aware layer on top of Kani*. Keep the property narrow (panic-freedom),
the bounds explicit, the UI honest about what was checked. Ship the CLI in ~90 days, get 50
real users, then expand (properties → Python via CrossHair/Z3 → functional correctness).
