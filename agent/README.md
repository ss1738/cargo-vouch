# vouch-agent, write a Rust function, then *prove* it panic-free

A dev agent that closes the loop around cargo-vouch:

```
spec ─▶ Claude writes a Rust fn ─▶ cargo-vouch (Kani BMC) ─▶ 🔴/🟡/⏱/⏭ → feed the
                                                            verdict + witness back
                                                          ─▶ ✅ VERIFIED → done
```

The point: an LLM confidently calls its own code "correct", cargo-vouch's whole
premise is that you shouldn't believe it. This agent makes an **independent
bounded model checker** the oracle. On a 🔴 BUG it feeds the *concrete
counterexample* ("reachable with input(s): empty vector" / "divide by zero at
-1, 0") back as the repair instruction, and it never returns a function it
couldn't prove panic-free.

## Setup

```bash
python3 -m venv .venv && .venv/bin/pip install anthropic
# cargo-vouch must be built (../cli) or on PATH, and Kani installed:
#   cargo install --locked kani-verifier && cargo kani setup
# Credentials: export ANTHROPIC_API_KEY=...   (or `ant auth login`)
```

## Use

```bash
.venv/bin/python vouch_agent.py "a function that returns the average of a slice of i32"
```

```console
── iteration 1/4 — asking claude-opus-4-8 ──
generated:
    fn average(xs: &[i32]) -> i32 { xs.iter().sum::<i32>() / xs.len() as i32 }
── verifying with cargo-vouch ──
    BUG          average  ← 0 (usize → empty vector)
── iteration 2/4 — asking claude-opus-4-8 ──
generated:
    fn average(xs: &[i32]) -> i32 {
        if xs.is_empty() { return 0; }
        xs.iter().map(|&x| x as i64).sum::<i64>() as i32 / xs.len() as i32
    }
── verifying with cargo-vouch ──
    VERIFIED     average
✅ PROVEN — the function verifies.
```

## Options

| flag | meaning |
|---|---|
| `--max-iters N` | repair rounds before giving up (default 4) |
| `--model ID` | Claude model (default `claude-opus-4-8`) |
| `--fail-on bug\|unguarded\|inconclusive` | which verdicts to treat as failures to repair (default `bug`, extreme-only overflows pass) |
| `--bound/--unwind/--str-bound/--str-unwind N` | passed through to cargo-vouch |
| `--out PATH` | write the final proven function to a file |
| `--cargo-vouch PATH` | explicit cargo-vouch binary |

Exit code `0` = proven, `1` = gave up unproven, `2` = setup/API error.

## Scope

The agent inherits cargo-vouch's niche: it's told to write **loop-light** functions
over supported param types. It shines on the class of code cargo-vouch verifies (arithmetic, indexing, `unwrap`/empty-collection safety), and honestly reports
INCONCLUSIVE (with a rewrite nudge) when a spec drags it into data-dependent
loops. See `../REAL_WORLD_VALIDATION.md`.
