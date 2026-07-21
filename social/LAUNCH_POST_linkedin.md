# LinkedIn launch post — cargo-vouch

## Caption (copy-paste) — unanimous across GPT-4o + Qwen + Kimi + Claude

I just launched cargo-vouch, an open-source Rust tool that proves a function can't panic. No tests to write, no annotations to add.

Point it at a .rs file and it builds a Kani proof harness for every function, then checks every input in bounds. Each function comes back with one of four verdicts:

🔴 BUG, with the exact input that triggers the panic
🟡 UNGUARDED, safe except at type extremes like i32::MAX
🟢 VERIFIED, provably panic-free
⏳ INCONCLUSIVE, when it cannot decide

Why I built it: AI writes Rust that passes cargo test, then panics on an edge input nobody tried. For example, numbers.iter().sum() looks fine until something hands it [i32::MAX, 1].

I ran it on 11 functions GPT-4o wrote and graded. GPT-4o's own grading got 4 of them wrong: it passed three that overflow and failed one that was fine. cargo-vouch called all four correctly, with the triggering input for each real panic.

The honest limit: it is built for loop-light code. Parsers and loop-heavy functions return INCONCLUSIVE, never a false pass. I would rather say that than oversell it.

If you write Rust, I would like your feedback.

cargo install cargo-vouch

#rustlang #opensource #formalverification

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
