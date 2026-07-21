// Week-1 spike: HAND-WRITTEN Kani harnesses for corpus functions.
// These are the pattern `cargo-vouch` will AUTO-GENERATE from the fn signature.
// Bounds: Vec length <= 3, loops unwound <= 5. Property: panic-freedom + overflow.
//
// Run (once Kani is installed):  cargo kani
//
// The signature -> harness mapping the generator implements:
//   i32            -> let x: i32 = kani::any();
//   Vec<i32>       -> bounded symbolic Vec (len assumed <= BOUND)
//   Option<i32>    -> kani::any()  (Some(any) | None)
//   docstring hint "non-empty" -> kani::assume!(len > 0)

// ---- helper: a bounded symbolic Vec<i32> (the core of harness synthesis) ----
fn any_bounded_vec(bound: usize) -> Vec<i32> {
    let len: usize = kani::any();
    kani::assume(len <= bound);
    let mut v: Vec<i32> = Vec::with_capacity(len);
    for _ in 0..len {
        v.push(kani::any());
    }
    v
}

// 06_find_max — SEEDED BUG: `.max().unwrap()` panics on empty vec.
// Expected: Kani finds the panic with input vec![] (len == 0).
fn find_max(numbers: Vec<i32>) -> i32 {
    *numbers.iter().max().unwrap()
}
#[kani::proof]
#[kani::unwind(5)]
fn verify_find_max() {
    let v = any_bounded_vec(3);
    let _ = find_max(v);            // Kani explores len==0 -> unwrap panic
}

// 01_sum_vec — LABELLED "correct" by the AI, but `i32` sum OVERFLOWS.
// Expected: Kani finds arithmetic overflow (e.g. [i32::MAX, 1]).
// This is the money demo: the verifier catches what the AI (and its label) missed.
fn sum_vec(numbers: Vec<i32>) -> i32 {
    numbers.iter().sum()
}
#[kani::proof]
#[kani::unwind(5)]
fn verify_sum_vec() {
    let v = any_bounded_vec(3);
    let _ = sum_vec(v);            // overflow on large elements
}

// 04_divide — SEEDED BUG: div-by-zero AND i32::MIN / -1 overflow.
// Expected: Kani finds both b==0 and (i32::MIN, -1).
fn divide(a: i32, b: i32) -> i32 {
    a / b
}
#[kani::proof]
fn verify_divide() {
    let a: i32 = kani::any();
    let b: i32 = kani::any();
    let _ = divide(a, b);         // b==0 panic; MIN/-1 overflow
}
