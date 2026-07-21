# cargo-vouch on realistic code (dogfood)

Not the labelled corpus — these are the kind of small utility modules real Rust
projects actually ship (`dogfood/`). Every verdict below is what the tool printed,
reported in full (no cherry-picking). Reproduce with `cargo-vouch dogfood/<file>.rs`.

## geometry.rs — integer geometry

| function | verdict | why |
|---|---|---|
| `manhattan(a, b)` | 🟡 UNGUARDED | `.abs()` overflows at `i32::MIN`; subtraction can overflow |
| `rect_area(w, h)` | 🟡 UNGUARDED | `w * h` overflows |
| `midpoint_x(a, b)` | 🟡 UNGUARDED | `a.0 + b.0` overflows before the `/2` |
| `scale(v, factor)` | 🟡 UNGUARDED | `v.0 * factor` overflows |

All four are the *classic* latent overflow — fine on normal inputs, unsound at the
extremes. cargo-vouch flags them as UNGUARDED (add a guard), not as false BUGs.

## pagination.rs — list-UI arithmetic

| function | verdict | why |
|---|---|---|
| `page_count(total, per_page)` | 🔴 **BUG** | **divide by zero when `per_page == 0`** — witness `total=-1, per_page=0` |
| `offset(page, per_page)` | 🟡 UNGUARDED | `page * per_page` overflows |
| `clamp_page(page, max)` | ✅ VERIFIED | provably panic-free |
| `items_on_page(...)` | 🟡 UNGUARDED | `page * per_page` overflows |

**`page_count` is a real, shippable bug** — a `per_page` of 0 (easy to reach from a
bad query param or config) panics the process. `cargo test` with sensible page sizes
would never surface it; the proof does, with the exact triggering input. And
`clamp_page` — deliberately written defensively — is *proven* safe, so you know the
guard actually works.

## stats.rs — numeric summaries over a slice

| function | verdict | why |
|---|---|---|
| `total(xs)` | 🟡 UNGUARDED `[MEASURED, 8s]` | `.sum()` overflows |
| `mean(xs)` | 🔴 **BUG** `[MEASURED, 13s]` | **divide-by-zero on an empty slice** (`sum / len`) |
| `maximum(xs)` | 🔴 **BUG** `[MEASURED, 116s]` | `.max().unwrap()` on an empty slice |
| `minimum(xs)` | 🔴 **BUG** `[MEASURED]` | `.min().unwrap()` on an empty slice (empty-vec witness) |
| `spread(xs)` | 🔴 **BUG** `[MEASURED]` | calls `maximum`/`minimum`, so panics on empty too (empty-vec witness) |
| `abs_total(xs)` | 🟡 UNGUARDED `[MEASURED, 7s]` | `x.abs()` overflows at `i32::MIN`; `.sum()` overflows |

Two more **real, shippable bugs**: a stats module that panics on empty input (`mean`
divides by zero; `maximum`/`minimum` unwrap `None`, and `spread` inherits it).
Empty-collection handling is the single most common latent panic in this kind of code,
and every one is caught with the witness. (`maximum`/`minimum`/`spread` are the slow
`.min()/.max().unwrap()` case — `maximum` alone is 116s, and all three together verified
in one whole-file run in ~26 min; every verdict here was run, none inferred.)

## An honest limitation this surfaced

`stats.rs` first came back **all INCONCLUSIVE** as a whole file. Root cause, measured:
`.iter().max()/.min().unwrap()` is *pathologically slow* in Kani — **~116s for a single
function**. A file's harnesses share one `cargo kani` invocation and (originally) one
120s timeout, so a slow function timed out the whole file and every verdict was lost.

Fixed in this session (commit `dfd432f`): the per-file timeout now **scales with the
function count** (`120s × n`, capped at 600s), so a fast function isn't starved by a slow
sibling. But the underlying truth stands and is worth stating plainly: **BMC is expensive
on some iterator patterns.** cargo-vouch is honest about it — a function it can't finish
proving is ⏱️ INCONCLUSIVE ("raise `--unwind`/wait", *not* a pass), never a false ✅.

## The loop closes: find → fix → *prove fixed*

The point of a verifier isn't just to complain — it's to tell you when you're done. I took
the pagination bugs, added exactly the guards the report asked for, and re-ran
(`dogfood/pagination_fixed.rs`):

```rust
fn page_count(total: i32, per_page: i32) -> i32 {
    if per_page <= 0 { return 0; }                    // guard the divide-by-zero
    total.saturating_add(per_page - 1) / per_page     // and the overflow
}
fn offset(page: i32, per_page: i32) -> i32 { page.saturating_mul(per_page) }
```

```console
$ cargo-vouch dogfood/pagination_fixed.rs
✅ VERIFIED  page_count   ✅ VERIFIED  offset   ✅ VERIFIED  clamp_page    # exit 0
```

🔴 BUG → guard added → ✅ VERIFIED. Not "the new tests pass" — *proven* panic-free for every
input in bounds. That's the difference from a test suite: it can tell you the bug is gone.

## Bottom line

Across 14 functions of realistic code, cargo-vouch found **five reachable panics that
`cargo test` would ship** (a divide-by-zero page calc, a divide-by-zero mean, three
empty-slice unwraps), flagged every extreme-only overflow as UNGUARDED rather than crying
wolf, and *proved* the one defensively-written function safe — while being upfront about
where verification is slow. That's the whole pitch: **prove, don't pray — and don't lie
about what you couldn't prove.**
