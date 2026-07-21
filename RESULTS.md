# cargo-aiv on realistic code (dogfood)

Not the labelled corpus — these are the kind of small utility modules real Rust
projects actually ship (`dogfood/`). Every verdict below is what the tool printed,
reported in full (no cherry-picking). Reproduce with `cargo-aiv dogfood/<file>.rs`.

## geometry.rs — integer geometry

| function | verdict | why |
|---|---|---|
| `manhattan(a, b)` | 🟡 UNGUARDED | `.abs()` overflows at `i32::MIN`; subtraction can overflow |
| `rect_area(w, h)` | 🟡 UNGUARDED | `w * h` overflows |
| `midpoint_x(a, b)` | 🟡 UNGUARDED | `a.0 + b.0` overflows before the `/2` |
| `scale(v, factor)` | 🟡 UNGUARDED | `v.0 * factor` overflows |

All four are the *classic* latent overflow — fine on normal inputs, unsound at the
extremes. cargo-aiv flags them as UNGUARDED (add a guard), not as false BUGs.

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
