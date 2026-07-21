//! End-to-end tests: run the actual built binary against fixtures and assert the
//! verdict + exit code. These invoke Kani, so they're `#[ignore]`d — the fast CI
//! (no Kani) skips them; run them where Kani is installed with:
//!
//!     cargo test --release -- --ignored
//!
//! `cargo test` builds the binary and exposes it via CARGO_BIN_EXE_cargo-vouch.

use std::fs;
use std::process::Command;

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_cargo-vouch"))
        .args(args)
        .output()
        .expect("failed to run cargo-vouch");
    let code = out.status.code().unwrap_or(-1);
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (code, text)
}

fn fixture(name: &str, src: &str) -> String {
    let p = std::env::temp_dir().join(format!("vouch-it-{name}.rs"));
    fs::write(&p, src).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
#[ignore]
fn selftest_passes_when_kani_present() {
    let (code, out) = run(&["--selftest"]);
    assert_eq!(code, 0, "selftest should pass: {out}");
    assert!(out.contains("PASS"), "{out}");
}

#[test]
#[ignore]
fn empty_vec_unwrap_is_bug_exit_1() {
    let f = fixture("bug", "fn f(v: Vec<i32>) -> i32 { v[0] }\n");
    let (code, out) = run(&[&f]);
    assert_eq!(code, 1, "a reachable panic must exit 1: {out}");
    assert!(out.contains("BUG"), "{out}");
}

#[test]
#[ignore]
fn identity_is_verified_exit_0() {
    let f = fixture("ok", "fn f(x: i32) -> i32 { x }\n");
    let (code, out) = run(&[&f]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("VERIFIED"), "{out}");
}

#[test]
#[ignore]
fn plain_sum_is_unguarded_not_bug() {
    let f = fixture("unguarded", "fn f(a: i32, b: i32) -> i32 { a + b }\n");
    let (code, out) = run(&[&f]);
    assert_eq!(code, 0, "unguarded overflow must not fail CI: {out}");
    assert!(out.contains("UNGUARDED"), "{out}");
}

#[test]
#[ignore]
fn prove_holds_and_violated() {
    let f = fixture(
        "abs",
        "fn f(x: i32) -> i32 { if x < 0 { -x } else { x } }\n",
    );
    let (c1, o1) = run(&["--prove", "result >= 0", &f]);
    assert_eq!(c1, 0, "provable postcondition should exit 0: {o1}");
    assert!(o1.contains("PROVEN"), "{o1}");
    let (c2, o2) = run(&["--prove", "result > 0", &f]);
    assert_eq!(c2, 1, "false postcondition should exit 1: {o2}");
    assert!(o2.contains("VIOLATED"), "{o2}");
}

#[test]
#[ignore]
fn unbounded_loop_is_inconclusive_not_bug() {
    // (1..=n).product() needs ~n unwinds; at the default bound it can't finish → not a BUG.
    let f = fixture("fact", "fn f(n: i32) -> i32 { (1..=n).product() }\n");
    let (code, out) = run(&[&f]);
    assert_eq!(
        code, 2,
        "unwinding-limited result is INCONCLUSIVE (exit 2): {out}"
    );
    assert!(out.contains("INCONCLUSIVE"), "{out}");
}
