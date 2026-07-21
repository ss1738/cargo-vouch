# LinkedIn launch post — cargo-vouch

## Caption (copy-paste)

An AI wrote a Rust function, passed `cargo test`, and shipped it.
It still panics on one input the tests never tried.

That gap is why I built cargo-vouch.

It is a cargo subcommand that proves a Rust function can't panic. You point it at a file, it writes a Kani proof harness for every function, and checks every input in bounds. No annotations, no test cases to write.

You get a verdict:
🔴 BUG, with the exact input that triggers it
🟡 UNGUARDED, overflows only at i32::MAX / MIN
✅ VERIFIED, provably panic-free

I ran it on 11 Rust functions GPT-4o wrote and graded as correct. It was wrong 4 times: three overflowed on edge inputs, one was actually fine but it flagged it anyway. cargo-vouch caught all four, and handed back the input that breaks each one.

The honest limit: it is for loop-light code, the arithmetic, indexing and unwraps where most of these bugs live. Point it at a parser and it returns INCONCLUSIVE. On two random crates from crates.io, 6 of 6 functions came back INCONCLUSIVE. It never fakes a pass.

Live and open source (MIT):
cargo install cargo-vouch

Built on Kani. Would love feedback from anyone writing Rust.

#Rust #AI #FormalVerification #DeveloperTools

---

## First comment (put the link here, not in the body)

Repo and how it works: https://github.com/ss1738/cargo-vouch

---

## Notes for posting
- The first two lines are the hook shown before "see more" — they have to earn the click.
- LinkedIn suppresses reach on posts with an outbound link in the body. Keep the link in the first comment (or edit it in after posting).
- Asset to attach — pick one:
  - Carousel (5 slides, document post): strong LinkedIn reach, no audio, no music decision needed. Best for posting now. Screenshot each slide from social/linkedin-carousel.html.
  - Video (landscape cargo-vouch-launch.mp4): more engaging, but resolve the music first.
- Post mid-morning UK time on a weekday for best reach.
