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
