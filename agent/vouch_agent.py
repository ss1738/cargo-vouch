#!/usr/bin/env python3
"""
vouch-agent — write a Rust function, then *prove* it panic-free.

The loop:

    spec ──▶ Claude writes a Rust fn ──▶ cargo-vouch (Kani BMC)
                                              │
              ┌───────────────────────────────┤
              │ 🔴 BUG / 🟡 UNGUARDED / ⏱ INCONCLUSIVE / ⏭ unsupported
              │   → feed the verdict + witness back, ask for a fix
              ▼
        ✅ VERIFIED  → done (proven, not just tested)

This is the capstone of cargo-vouch: instead of trusting an LLM's own claim that
its code is correct, an independent bounded model checker verifies it, and the
concrete counterexample (e.g. "empty vector", "divide by zero at -1, 0") is fed
back as the repair instruction. It never ships a function it couldn't prove.

Requires: cargo-vouch on PATH (or --cargo-vouch), Kani installed, and Anthropic
credentials (ANTHROPIC_API_KEY, or an `ant auth login` profile).
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

import anthropic

MODEL = "claude-opus-4-8"

SYSTEM_PROMPT = """\
You are a Rust engineer. Given a specification, write ONE safe Rust function \
(plus any same-file structs, enums, or small helper functions it needs) that \
satisfies it AND is provably free of panics and integer overflow.

Your code is checked by a bounded model checker (Kani, via `cargo-vouch`), not a \
test suite — it explores every input within bounds. Write so it can prove your \
code safe:

- Safe Rust only. No `unsafe`, no external crates, no macros beyond std.
- Keep it LOOP-LIGHT. Data-dependent or unbounded loops (`while x > 0`, parsing \
loops, nested iteration) are out of the checker's reach and come back \
INCONCLUSIVE. Prefer straight-line arithmetic, indexing, and single small \
operations. If the spec truly needs a loop, keep the bound tiny and fixed.
- Parameter types must be within scope: scalar ints (i8..u64, isize/usize), \
bool, `Vec<int>`, `Option<int>`, int slices (`&[int]`), `&str`/`String`, tuples \
of ints, and same-file structs/enums whose fields are these. Nothing else.
- GUARD every partial operation: check divisors before dividing, check emptiness \
before indexing/`.unwrap()`, and use `checked_`/`saturating_`/`wrapping_` \
arithmetic (or explicit bounds guards) instead of raw `+ - *` that can overflow.
- Return a total function: no input in bounds should panic.

Output ONLY a single fenced ```rust code block containing the function (and any \
supporting types). No prose, no explanation outside the code block.\
"""


def find_cargo_vouch(explicit: str | None) -> str:
    """Locate the cargo-vouch binary: explicit path, repo debug build, or PATH."""
    if explicit:
        return explicit
    here = os.path.dirname(os.path.abspath(__file__))
    local = os.path.join(here, "..", "cli", "target", "debug", "cargo-vouch")
    if os.path.exists(local):
        return os.path.abspath(local)
    return "cargo-vouch"  # rely on PATH


def extract_code(text: str) -> str:
    """Pull the Rust source out of a ```rust fence (or fall back to raw text)."""
    m = re.search(r"```(?:rust)?\s*\n(.*?)```", text, re.DOTALL)
    return (m.group(1) if m else text).strip()


def run_cargo_vouch(binary: str, src: str, flags: list[str]) -> dict:
    """Write `src` to a temp .rs, run `cargo-vouch --json`, return the parsed result."""
    with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as f:
        f.write(src)
        path = f.name
    try:
        proc = subprocess.run(
            [binary, *flags, "--json", path],
            capture_output=True,
            text=True,
        )
        out = proc.stdout.strip()
        # The last line is the JSON object; earlier lines may be progress text.
        line = out.splitlines()[-1] if out else ""
        try:
            return json.loads(line)
        except json.JSONDecodeError:
            return {
                "results": [
                    {
                        "name": "<file>",
                        "verdict": "ERROR",
                        "checks": [(proc.stderr or out or "no output").strip()[:500]],
                        "witness": [],
                    }
                ],
                "summary": {},
            }
    finally:
        os.unlink(path)


# Verdicts the agent must repair, and how each reads back to the model.
def feedback_for(results: list[dict], fail_on: str) -> str | None:
    """Return a repair instruction if any function isn't acceptable, else None."""
    fail_unguarded = fail_on in ("unguarded", "inconclusive")
    lines = []
    verified_any = False
    for r in results:
        name, v = r["name"], r["verdict"]
        checks = "; ".join(r.get("checks") or [])
        witness = ", ".join(r.get("witness") or [])
        if v == "VERIFIED":
            verified_any = True
        elif v == "BUG":
            w = f"  reachable with input(s): {witness}" if witness else ""
            lines.append(f"- `{name}`: 🔴 BUG — {checks}.{w}")
        elif v == "UNGUARDED":
            if fail_unguarded:
                lines.append(
                    f"- `{name}`: 🟡 UNGUARDED — {checks}. Overflows only at extremes "
                    f"(e.g. i32::MAX); guard it with checked_/saturating_ arithmetic."
                )
            else:
                verified_any = True  # acceptable at the default gate
        elif v == "INCONCLUSIVE":
            lines.append(
                f"- `{name}`: ⏱ INCONCLUSIVE — the checker couldn't finish, almost "
                f"always because the function iterates. Rewrite it loop-light "
                f"(remove data-dependent loops; keep it straight-line)."
            )
        elif v == "UNSUPPORTED":
            lines.append(
                f"- `{name}`: ⏭ out of scope — {checks}. Use only supported param "
                f"types (scalar ints, Vec<int>, Option<int>, &[int], &str/String, "
                f"tuples, same-file structs/enums)."
            )
        else:  # ERROR / parse failure
            lines.append(
                f"- `{name}`: did not compile/parse — {checks}. Return valid, "
                f"self-contained Rust."
            )
    if lines:
        return "The verifier found problems:\n" + "\n".join(lines) + (
            "\n\nFix these and output ONLY the corrected ```rust block."
        )
    if not verified_any:
        return (
            "The verifier produced no ✅ VERIFIED result. Make sure the function "
            "is provably panic-free and re-emit it."
        )
    return None  # all good


def assistant_text(message) -> str:
    return "".join(b.text for b in message.content if b.type == "text")


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Write a Rust function and prove it panic-free with cargo-vouch."
    )
    ap.add_argument("spec", help="Natural-language description of the function to write")
    ap.add_argument("--max-iters", type=int, default=4, help="Max repair rounds (default 4)")
    ap.add_argument("--model", default=MODEL, help=f"Claude model (default {MODEL})")
    ap.add_argument("--cargo-vouch", help="Path to the cargo-vouch binary")
    ap.add_argument("--fail-on", choices=["bug", "unguarded", "inconclusive"], default="bug",
                    help="Which verdicts count as failures to repair (default: bug)")
    ap.add_argument("--bound", type=int, help="cargo-vouch --bound")
    ap.add_argument("--unwind", type=int, help="cargo-vouch --unwind")
    ap.add_argument("--str-bound", type=int, help="cargo-vouch --str-bound")
    ap.add_argument("--str-unwind", type=int, help="cargo-vouch --str-unwind")
    ap.add_argument("--out", help="Write the final proven function to this path")
    args = ap.parse_args()

    binary = find_cargo_vouch(args.cargo_vouch)
    flags: list[str] = ["--fail-on", args.fail_on]
    for name in ("bound", "unwind", "str_bound", "str_unwind"):
        val = getattr(args, name)
        if val is not None:
            flags += [f"--{name.replace('_', '-')}", str(val)]

    try:
        client = anthropic.Anthropic()
    except Exception as e:  # pragma: no cover - construction rarely fails
        print(f"error: could not init Anthropic client: {e}", file=sys.stderr)
        return 2

    messages = [{"role": "user", "content": f"Specification:\n{args.spec}"}]
    last_code = ""

    for it in range(1, args.max_iters + 1):
        print(f"\n── iteration {it}/{args.max_iters} — asking {args.model} ──")
        try:
            resp = client.messages.create(
                model=args.model,
                max_tokens=16000,
                thinking={"type": "adaptive"},
                output_config={"effort": "high"},
                system=SYSTEM_PROMPT,
                messages=messages,
            )
        except anthropic.APIError as e:
            print(f"error: Anthropic API call failed: {e}", file=sys.stderr)
            return 2

        if resp.stop_reason == "refusal":
            print("model refused this request; stopping.", file=sys.stderr)
            return 2

        text = assistant_text(resp)
        code = extract_code(text)
        last_code = code
        print("generated:\n" + "\n".join("    " + l for l in code.splitlines()))

        print(f"── verifying with cargo-vouch ({binary}) ──")
        result = run_cargo_vouch(binary, code, flags)
        for r in result.get("results", []):
            w = ("  ← " + ", ".join(r["witness"])) if r.get("witness") else ""
            print(f"    {r['verdict']:<12} {r['name']}{w}")

        fb = feedback_for(result.get("results", []), args.fail_on)
        if fb is None:
            print("\n✅ PROVEN — the function verifies. Final code:\n")
            print(code)
            if args.out:
                with open(args.out, "w") as f:
                    f.write(code + "\n")
                print(f"\nwritten to {args.out}")
            return 0

        # Feed the verdict back and iterate.
        messages.append({"role": "assistant", "content": text})
        messages.append({"role": "user", "content": fb})

    print(
        f"\n⚠ gave up after {args.max_iters} iterations without a full proof. "
        f"Last attempt above; rerun with --max-iters higher or adjust the spec.",
        file=sys.stderr,
    )
    if args.out and last_code:
        with open(args.out, "w") as f:
            f.write(last_code + "\n")
    return 1


if __name__ == "__main__":
    sys.exit(main())
