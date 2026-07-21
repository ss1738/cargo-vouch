// roman-0.2.1 — the 5 library functions verbatim (crate-level doc attr and the
// crate's own #[test] fns removed; static tables kept). Source unmodified otherwise.

static ROMAN: &[(char, i32)] = &[
    ('I', 1),
    ('V', 5),
    ('X', 10),
    ('L', 50),
    ('C', 100),
    ('D', 500),
    ('M', 1000),
];
static ROMAN_PAIRS: &[(&str, i32)] = &[
    ("M", 1000),
    ("CM", 900),
    ("D", 500),
    ("CD", 400),
    ("C", 100),
    ("XC", 90),
    ("L", 50),
    ("XL", 40),
    ("X", 10),
    ("IX", 9),
    ("V", 5),
    ("IV", 4),
    ("I", 1),
];

pub static MAX: i32 = 3999;

pub fn to(n: i32) -> Option<String> {
    if n <= 0 || n > MAX {
        return None;
    }
    let mut out = String::new();
    let mut n = n;
    for &(name, value) in ROMAN_PAIRS.iter() {
        while n >= value {
            n -= value;
            out.push_str(name);
        }
    }
    assert!(n == 0);
    Some(out)
}

pub fn to_lower(n: i32) -> Option<String> {
    to(n).map(|mut s| {
        s.make_ascii_lowercase();
        s
    })
}

pub fn from(txt: &str) -> Option<i32> {
    let n = from_lax(txt)?;
    match to(n) {
        Some(ref x) if *x == txt => Some(n),
        _ => None,
    }
}

pub fn from_lower(txt: &str) -> Option<i32> {
    let n = from_lax(txt)?;
    match to_lower(n) {
        Some(ref x) if *x == txt => Some(n),
        _ => None,
    }
}

fn from_lax(txt: &str) -> Option<i32> {
    let (mut n, mut max) = (0, 0);
    for c in txt.chars().rev() {
        let c = c.to_ascii_uppercase();
        let it = ROMAN.iter().find(|x| {
            let &(ch, _) = *x;
            ch == c
        });
        let &(_, val) = it?;
        if val < max {
            n -= val;
        } else {
            n += val;
            max = val;
        }
    }
    Some(n)
}
