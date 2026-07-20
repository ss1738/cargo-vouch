#!/usr/bin/env python3
"""cargo-aiv precondition-gap classifier (v0) — the anti-cry-wolf engine.

Verify each function TWICE:
  - strict    : inputs range over all of i32  (finds every reachable panic)
  - realistic : scalar/vec values bounded to a normal range [-1000, 1000]

Classification:
  strict FAIL + realistic FAIL  -> BUG           (reachable on ordinary input — real)
  strict FAIL + realistic PASS  -> UNGUARDED     (only overflows at i32::MAX/MIN — downgrade)
  strict PASS                   -> VERIFIED

This is the difference between a tool devs trust and one they uninstall after the
first false alarm on `let x: i32 = a + b`.
"""
import os
import re
import subprocess
import sys

from gen_harness import generate

ENV = {**os.environ, "PATH": f"{os.path.expanduser('~/.cargo/bin')}:{os.environ.get('PATH','')}"}


def run_kani(outdir):
    """Return {fn_name: (verdict, [failed_checks])} from `cargo kani` in outdir."""
    p = subprocess.run(["cargo", "kani"], cwd=outdir, env=ENV, capture_output=True, text=True)
    out, cur, res = p.stdout + p.stderr, None, {}
    for ln in out.splitlines():
        m = re.search(r"Checking harness verify_(\w+)", ln)
        if m:
            cur = m.group(1); res[cur] = ["", []]
        elif cur and "Failed Checks:" in ln:
            res[cur][1].append(ln.split("Failed Checks:", 1)[1].strip())
        elif cur and "VERIFICATION:-" in ln:
            res[cur][0] = "FAILED" if "FAILED" in ln else "PASS"
    return {k: (v[0], v[1]) for k, v in res.items()}


def main():
    print("Generating strict + realistic harnesses…")
    generate("spike2", realistic=False)
    generate("spike3", realistic=True)
    print("Running Kani (strict)…"); strict = run_kani("spike2")
    print("Running Kani (realistic)…"); realistic = run_kani("spike3")

    print("\n" + "=" * 74)
    print(f"{'FUNCTION':<16}{'VERDICT':<14}DETAIL")
    print("=" * 74)
    bugs = unguarded = verified = 0
    for fn in sorted(strict):
        s_fail = strict[fn][0] == "FAILED"
        r_fail = realistic.get(fn, ("", []))[0] == "FAILED"
        if not s_fail:
            verdict, detail, verified = "✅ VERIFIED", "panic-free for Vec≤3, |val|≤1000", verified + 1
        elif r_fail:
            verdict, detail, bugs = "🔴 BUG", "; ".join(realistic[fn][1])[:44], bugs + 1
        else:
            verdict, detail, unguarded = "🟡 UNGUARDED", "overflow only at i32::MAX/MIN — add a guard", unguarded + 1
        print(f"{fn:<16}{verdict:<14}{detail}")
    print("=" * 74)
    print(f"{bugs} real BUG(s) devs must fix · {unguarded} UNGUARDED (extreme-input only) · {verified} VERIFIED")
    print("\nThe UNGUARDED downgrade is the point: sum_vec/increment_all/square_sum stop")
    print("crying wolf, while empty-vec unwraps and divide-by-zero stay flagged as real bugs.")


if __name__ == "__main__":
    main()
