//! cargo-aiv — prove AI-generated Rust is panic-free (don't just test it).
//!
//! Parses a `.rs` file with `syn`, auto-generates a Kani proof harness for every
//! top-level function, runs bounded model checking in TWO modes (strict +
//! realistic), and reports per function:
//!   🔴 BUG          panic reachable on ordinary input — fix it
//!   🟡 UNGUARDED    overflow only at i32::MAX/MIN — add a guard (not a false alarm)
//!   ✅ VERIFIED     panic-free within bounds
//!   ⏱️  INCONCLUSIVE didn't finish within the per-mode timeout
//!
//!   cargo-aiv <file.rs | dir>...      verify a file, many files, or a whole dir
//!   cargo-aiv --prove '<expr>' <file> prove a postcondition over `result`/inputs
//!   cargo-aiv --emit <file.rs>        print the generated harness, don't run
//!   cargo-aiv --json <path>...        machine-readable {results, summary}
//!   cargo-aiv --selftest              check the Kani wiring is trustworthy
//!   cargo-aiv --bound N / --unwind N  tune the BMC depth
//!   cargo-aiv --fail-on <level>       gate strictness: bug|unguarded|inconclusive

use quote::quote;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering};

// Bound (max Vec/slice length) and unwind (loop unroll depth) are user-tunable via
// --bound / --unwind, set once in main() before any verification. RANGE stays const:
// it defines the strict-vs-realistic split that the UNGUARDED classification rests on.
static BOUND_A: AtomicUsize = AtomicUsize::new(3);
static UNWIND_A: AtomicU32 = AtomicU32::new(5);
// Symbolic strings are ~10x more expensive under BMC than Vec/int params (UTF-8
// decode + Unicode tables), so they get their OWN bound, defaulted low and
// decoupled from --bound. Length ≤1 still covers the empty-string case — where
// nearly every string panic lives (.unwrap()/.parse()/indexing) — and solves fast;
// at the shared Vec bound (3) even a trivial string bug times out to INCONCLUSIVE.
static STR_BOUND_A: AtomicUsize = AtomicUsize::new(1);
// Measured: string cost under BMC is dominated by the UNWIND depth (UTF-8 decode +
// from_utf8 validation loops), not the length. At the default unwind of 5 even a
// one-char symbolic string times out; at unwind 2 the same harness resolves in ~30s.
// So string-bearing harnesses get their own low unwind, decoupled from --unwind.
// A too-low unwind can only yield INCONCLUSIVE (unwinding assertion), never a false
// pass — so this is safe: it trades some string-loop depth for a usable default.
static STR_UNWIND_A: AtomicU32 = AtomicU32::new(2);
fn bound() -> usize {
    BOUND_A.load(Ordering::Relaxed)
}
fn str_bound() -> usize {
    STR_BOUND_A.load(Ordering::Relaxed)
}
fn unwind() -> u32 {
    UNWIND_A.load(Ordering::Relaxed)
}
fn str_unwind() -> u32 {
    STR_UNWIND_A.load(Ordering::Relaxed)
}

/// True if this param type binds a symbolic string (`&str` or `String`) — such a
/// harness uses the lower `str_unwind()` instead of `unwind()`.
fn is_string_type(ty: &syn::Type) -> bool {
    if let syn::Type::Reference(r) = ty {
        if let syn::Type::Path(tp) = &*r.elem {
            if tp.path.is_ident("str") {
                return true;
            }
        }
    }
    matches!(path_head(ty), Some((b, a)) if b == "String" && a.is_empty())
}

/// True if this param binds a symbolic `Vec`/slice (`Vec<_>`, `&[_]`, `&mut [_]`,
/// `&Vec<_>`) — its symbolic length shows up in the witness as a `usize` token.
fn is_vec_type(ty: &syn::Type) -> bool {
    if matches!(path_head(ty), Some((b, _)) if b == "Vec") {
        return true;
    }
    if let syn::Type::Reference(r) = ty {
        if matches!(&*r.elem, syn::Type::Slice(_)) {
            return true;
        }
        if matches!(path_head(&r.elem), Some((b, _)) if b == "Vec") {
            return true;
        }
    }
    false
}

/// How to read a `usize` witness token: a symbolic length can be a `Vec`/slice
/// length or a string length, and the raw token can't tell them apart. We resolve
/// it from the function's param types instead.
#[derive(Clone, Copy)]
enum LenHint {
    Vector,    // only Vec/slice collections → "empty vector"
    Str,       // only strings → "empty string"
    Ambiguous, // both present → neutral "empty collection" (can't disambiguate positionally)
}

/// Pick the witness length-noun for a function from its params.
fn len_hint_for(func: &syn::ItemFn) -> LenHint {
    let (mut has_vec, mut has_str) = (false, false);
    for arg in &func.sig.inputs {
        if let syn::FnArg::Typed(pt) = arg {
            if is_string_type(&pt.ty) {
                has_str = true;
            } else if is_vec_type(&pt.ty) {
                has_vec = true;
            }
        }
    }
    match (has_str, has_vec) {
        (true, false) => LenHint::Str,
        (true, true) => LenHint::Ambiguous,
        _ => LenHint::Vector,
    }
}

/// Per-function length-noun hints for a whole source file (name → hint).
fn hints_for_file(src: &str) -> BTreeMap<String, LenHint> {
    let mut m = BTreeMap::new();
    if let Ok(file) = syn::parse_file(src) {
        for it in &file.items {
            if let syn::Item::Fn(f) = it {
                m.insert(f.sig.ident.to_string(), len_hint_for(f));
            }
        }
    }
    m
}

// CI rigor dial (--fail-on): 0 = BUG only (default), 1 = + UNGUARDED, 2 = + INCONCLUSIVE.
static FAIL_ON: AtomicU8 = AtomicU8::new(0);
/// Does this verdict fail the gate at the current --fail-on level?
fn fails(v: &Verdict) -> bool {
    let level = FAIL_ON.load(Ordering::Relaxed);
    match v {
        Verdict::Bug(..) => true,
        Verdict::Unguarded(_) => level >= 1,
        Verdict::Inconclusive => level >= 2,
        _ => false, // VERIFIED / UNSUPPORTED never fail
    }
}
const RANGE: i64 = 1000;
/// Per-mode wall-clock cap. Kani can run for minutes on adapter-heavy functions;
/// past this we report ⏱️ INCONCLUSIVE rather than hang a CI job forever.
const TIMEOUT_SECS: u64 = 120;
/// Upper bound on a whole-file run's budget (per mode), so a big file stays bounded.
const TIMEOUT_MAX_SECS: u64 = 600;
const SCALARS: &[&str] = &[
    "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize", "bool",
];
const G: &str = "\x1b[32m";
const R: &str = "\x1b[31m";
const Y: &str = "\x1b[33m";
const B: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const X: &str = "\x1b[0m";

fn helper() -> String {
    format!(
        r#"// auto-generated by cargo-aiv
fn any_bounded_vec<T: kani::Arbitrary>(bound: usize) -> Vec<T> {{
    let len: usize = kani::any();
    kani::assume(len <= bound);
    let mut v: Vec<T> = Vec::with_capacity(len);
    for _ in 0..len {{ v.push(kani::any()); }}
    v
}}
fn any_bounded_vec_ranged(bound: usize) -> Vec<i32> {{
    let len: usize = kani::any();
    kani::assume(len <= bound);
    let mut v: Vec<i32> = Vec::with_capacity(len);
    for _ in 0..len {{ let x: i32 = kani::any(); kani::assume(x >= -{RANGE} && x <= {RANGE}); v.push(x); }}
    v
}}
fn any_bounded_string(bound: usize) -> String {{
    // ASCII bytes are always valid UTF-8, so the string is well-formed by construction
    // (the from_utf8 unwrap is provably safe — no harness-side panic).
    let len: usize = kani::any();
    kani::assume(len <= bound);
    let mut bytes: Vec<u8> = Vec::with_capacity(len);
    for _ in 0..len {{ let b: u8 = kani::any(); kani::assume(b < 128); bytes.push(b); }}
    String::from_utf8(bytes).unwrap()
}}"#
    )
}

fn path_head(ty: &syn::Type) -> Option<(String, Vec<syn::Type>)> {
    if let syn::Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            let mut args = Vec::new();
            if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                for a in &ab.args {
                    if let syn::GenericArgument::Type(t) = a {
                        args.push(t.clone());
                    }
                }
            }
            return Some((seg.ident.to_string(), args));
        }
    }
    None
}

fn scalar_int(ty: &syn::Type) -> Option<String> {
    let (base, args) = path_head(ty)?;
    (args.is_empty() && SCALARS.contains(&base.as_str())).then_some(base)
}

/// A bounded symbolic `Vec<et>` binding line. `mutable` makes it `let mut`.
fn vec_binding(name: &str, et: &str, realistic: bool, mutable: bool) -> String {
    let m = if mutable { "mut " } else { "" };
    let bound = bound();
    if realistic && et == "i32" {
        format!("    let {m}{name}: Vec<i32> = any_bounded_vec_ranged({bound});")
    } else {
        format!("    let {m}{name}: Vec<{et}> = any_bounded_vec::<{et}>({bound});")
    }
}

/// Map a parameter to (binding line(s), argument expression at the call site).
/// The arg expr differs from the name for by-reference params (`&v`, `&mut v`).
/// `types` holds same-file struct/enum defs so a struct or enum param can be
/// synthesized field-by-field / variant-by-variant (recursively).
fn map_input(
    name: &str,
    ty: &syn::Type,
    realistic: bool,
    types: &Types,
) -> Option<(String, String)> {
    if let Some(s) = scalar_int(ty) {
        let mut line = format!("    let {name}: {s} = kani::any();");
        if realistic && s.starts_with('i') {
            line += &format!("\n    kani::assume({name} >= -{RANGE} && {name} <= {RANGE});");
        } else if realistic && s.starts_with('u') {
            line += &format!("\n    kani::assume({name} <= {RANGE});");
        }
        return Some((line, name.to_string()));
    }
    // &str — bind a symbolic ASCII String and pass `&s` (coerces to &str).
    if let syn::Type::Reference(r) = ty {
        if let syn::Type::Path(tp) = &*r.elem {
            if tp.path.is_ident("str") {
                let line =
                    format!("    let {name}: String = any_bounded_string({});", str_bound());
                return Some((line, format!("&{name}")));
            }
        }
    }
    // &[int] / &mut [int] — bind a symbolic Vec and pass it by reference (a `&Vec`
    // coerces to `&[T]`). Covers slice params, extremely common in AI Rust.
    if let syn::Type::Reference(r) = ty {
        if let syn::Type::Slice(sl) = &*r.elem {
            let et = scalar_int(&sl.elem)?;
            let mutable = r.mutability.is_some();
            let arg = if mutable {
                format!("&mut {name}")
            } else {
                format!("&{name}")
            };
            return Some((vec_binding(name, &et, realistic, mutable), arg));
        }
        // &Vec<int> / &mut Vec<int>
        if let Some(("Vec", inner)) = path_head(&r.elem).as_ref().map(|(b, a)| (b.as_str(), a)) {
            let et = scalar_int(inner.first()?)?;
            let mutable = r.mutability.is_some();
            let arg = if mutable {
                format!("&mut {name}")
            } else {
                format!("&{name}")
            };
            return Some((vec_binding(name, &et, realistic, mutable), arg));
        }
    }
    // (i32, u8, ...) — a tuple of scalar ints. Bind one kani::any() per element,
    // with per-element realistic assumes on the tuple fields.
    if let syn::Type::Tuple(tup) = ty {
        if tup.elems.is_empty() {
            return None; // unit () — nothing to synthesize
        }
        let mut types = Vec::new();
        for el in &tup.elems {
            types.push(scalar_int(el)?); // any non-scalar element ⇒ unsupported
        }
        let anys: Vec<&str> = types.iter().map(|_| "kani::any()").collect();
        let tystr = format!("({})", types.join(", "));
        let mut line = format!("    let {name}: {tystr} = ({});", anys.join(", "));
        if realistic {
            for (i, s) in types.iter().enumerate() {
                if s.starts_with('i') {
                    line += &format!(
                        "\n    kani::assume({name}.{i} >= -{RANGE} && {name}.{i} <= {RANGE});"
                    );
                } else if s.starts_with('u') {
                    line += &format!("\n    kani::assume({name}.{i} <= {RANGE});");
                }
            }
        }
        return Some((line, name.to_string()));
    }
    let (base, args) = path_head(ty)?;
    match (base.as_str(), args.as_slice()) {
        ("String", []) => {
            let line = format!("    let {name}: String = any_bounded_string({});", str_bound());
            Some((line, name.to_string()))
        }
        ("Vec", [inner]) => {
            let et = scalar_int(inner)?;
            Some((vec_binding(name, &et, realistic, false), name.to_string()))
        }
        ("Option", [inner]) => {
            let et = scalar_int(inner)?;
            let tystr = quote!(#ty).to_string().replace(' ', "");
            let mut line = format!("    let {name}: {tystr} = kani::any();");
            // Clamp the Some(_) payload in realistic mode too — otherwise an Option
            // overflow at i32::MAX is misreported as BUG instead of UNGUARDED.
            if realistic && et.starts_with('i') {
                line += &format!(
                    "\n    if let Some(v) = {name} {{ kani::assume(v >= -{RANGE} && v <= {RANGE}); }}"
                );
            } else if realistic && et.starts_with('u') {
                line += &format!("\n    if let Some(v) = {name} {{ kani::assume(v <= {RANGE}); }}");
            }
            Some((line, name.to_string()))
        }
        // A same-file struct or enum: synthesize it. Structs win if a name is both
        // (it can't be, but keep the order deterministic).
        _ if args.is_empty() => types
            .structs
            .get(base.as_str())
            .and_then(|fields| synth_struct(name, &base, fields, realistic, types))
            .or_else(|| {
                types
                    .enums
                    .get(base.as_str())
                    .and_then(|variants| synth_enum(name, &base, variants, realistic, types))
            }),
        _ => None,
    }
}

/// Named-field struct defs collected from the source file: name → [(field, type)].
type StructMap = BTreeMap<String, Vec<(String, syn::Type)>>;
/// Enum defs collected from the source file: name → [(variant, its fields)].
type EnumMap = BTreeMap<String, Vec<(String, syn::Fields)>>;

/// The same-file type registry threaded through synthesis (structs + enums).
#[derive(Default)]
struct Types {
    structs: StructMap,
    enums: EnumMap,
}

/// Collect every same-file struct (named fields) and enum into a [`Types`] registry.
fn collect_types(file: &syn::File) -> Types {
    let mut t = Types::default();
    for it in &file.items {
        match it {
            syn::Item::Struct(s) => {
                if let syn::Fields::Named(named) = &s.fields {
                    let fields = named
                        .named
                        .iter()
                        .filter_map(|f| f.ident.as_ref().map(|id| (id.to_string(), f.ty.clone())))
                        .collect();
                    t.structs.insert(s.ident.to_string(), fields);
                }
            }
            syn::Item::Enum(e) => {
                let variants = e
                    .variants
                    .iter()
                    .map(|v| (v.ident.to_string(), v.fields.clone()))
                    .collect();
                t.enums.insert(e.ident.to_string(), variants);
            }
            _ => {}
        }
    }
    t
}

/// Synthesize a symbolic struct: bind each field to a synthetic local (reusing
/// `map_input`, so nested structs/enums/strings/Vecs all work), then construct the
/// value. Returns None if ANY field is an unsupported or by-reference type — the
/// struct bounces cleanly, never a wrong answer. Recursion is bounded: a cyclic type
/// needs `Box`/`Rc`/`Vec` indirection, which `map_input` rejects, so it terminates.
fn synth_struct(
    name: &str,
    base: &str,
    fields: &[(String, syn::Type)],
    realistic: bool,
    types: &Types,
) -> Option<(String, String)> {
    if fields.is_empty() {
        return None; // a fieldless struct has nothing symbolic to bind
    }
    let mut lines = Vec::new();
    let mut inits = Vec::new();
    for (fname, fty) in fields {
        let local = format!("{name}_{fname}");
        let (line, arg) = map_input(&local, fty, realistic, types)?;
        if arg != local {
            return None; // by-reference/borrow field (e.g. &[T]) — not owned, reject
        }
        lines.push(line);
        inits.push(format!("{fname}: {local}"));
    }
    lines.push(format!(
        "    let {name}: {base} = {base} {{ {} }};",
        inits.join(", ")
    ));
    Some((lines.join("\n"), name.to_string()))
}

/// Build the constructor expression for one enum variant, binding each of its
/// fields symbolically inside a block. None if any field is unsupported/by-ref.
fn variant_ctor(
    name: &str,
    base: &str,
    vi: usize,
    vname: &str,
    fields: &syn::Fields,
    realistic: bool,
    types: &Types,
) -> Option<String> {
    // Flatten a field binding (may be multi-statement) onto one line for the arm.
    let flat = |line: String| line.replace('\n', " ").split_whitespace().collect::<Vec<_>>().join(" ");
    match fields {
        syn::Fields::Unit => Some(format!("{base}::{vname}")),
        syn::Fields::Unnamed(un) => {
            let mut lines = Vec::new();
            let mut locals = Vec::new();
            for (j, f) in un.unnamed.iter().enumerate() {
                let local = format!("{name}_{vi}_{j}");
                let (line, arg) = map_input(&local, &f.ty, realistic, types)?;
                if arg != local {
                    return None;
                }
                lines.push(flat(line));
                locals.push(local);
            }
            Some(format!(
                "{{ {} {base}::{vname}({}) }}",
                lines.join(" "),
                locals.join(", ")
            ))
        }
        syn::Fields::Named(nm) => {
            let mut lines = Vec::new();
            let mut inits = Vec::new();
            for f in &nm.named {
                let fname = f.ident.as_ref()?.to_string();
                let local = format!("{name}_{vi}_{fname}");
                let (line, arg) = map_input(&local, &f.ty, realistic, types)?;
                if arg != local {
                    return None;
                }
                lines.push(flat(line));
                inits.push(format!("{fname}: {local}"));
            }
            Some(format!(
                "{{ {} {base}::{vname} {{ {} }} }}",
                lines.join(" "),
                inits.join(", ")
            ))
        }
    }
}

/// Synthesize a symbolic enum: pick a variant with a nondeterministic selector and
/// construct it (binding that variant's fields). CBMC explores every arm, so every
/// variant is verified. None if any variant has an unsupported/by-ref field.
fn synth_enum(
    name: &str,
    base: &str,
    variants: &[(String, syn::Fields)],
    realistic: bool,
    types: &Types,
) -> Option<(String, String)> {
    if variants.is_empty() {
        return None; // uninhabited enum — nothing to construct
    }
    let n = variants.len();
    let mut arms = Vec::new();
    for (i, (vname, fields)) in variants.iter().enumerate() {
        let ctor = variant_ctor(name, base, i, vname, fields, realistic, types)?;
        // Last variant is the `_` catch-all so the match is exhaustive over `sel % n`.
        let pat = if i + 1 == n {
            "_".to_string()
        } else {
            i.to_string()
        };
        arms.push(format!("        {pat} => {ctor},"));
    }
    let body = format!(
        "    let {name}_sel: usize = kani::any();\n    let {name}: {base} = match {name}_sel % {n} {{\n{}\n    }};",
        arms.join("\n")
    );
    Some((body, name.to_string()))
}

/// True if a type binds (recursively, through struct fields and enum variants) a
/// symbolic string — used to give string-bearing harnesses the lower str_unwind
/// even when the string is nested inside a struct/enum param.
fn type_has_string(ty: &syn::Type, types: &Types) -> bool {
    if is_string_type(ty) {
        return true;
    }
    if let Some((base, args)) = path_head(ty) {
        if args.is_empty() {
            if let Some(fields) = types.structs.get(&base) {
                return fields.iter().any(|(_, fty)| type_has_string(fty, types));
            }
            if let Some(variants) = types.enums.get(&base) {
                return variants
                    .iter()
                    .any(|(_, f)| f.iter().any(|field| type_has_string(&field.ty, types)));
            }
        }
    }
    false
}

/// Generate just the Kani proof harness for ONE function. Err(msg) if any param
/// is unsupported. `postcond`, if set, becomes the proof obligation instead of the
/// default panic-freedom check.
fn harness_for(
    func: &syn::ItemFn,
    realistic: bool,
    postcond: Option<&str>,
    types: &Types,
) -> Result<String, String> {
    let name = func.sig.ident.to_string();
    let mut inputs = Vec::new();
    let mut args = Vec::new();
    let mut stringy = false;
    for arg in &func.sig.inputs {
        match arg {
            syn::FnArg::Typed(pt) => {
                let pname = match &*pt.pat {
                    syn::Pat::Ident(pi) => pi.ident.to_string(),
                    _ => return Err(format!("unsupported parameter pattern in `{name}`")),
                };
                stringy |= type_has_string(&pt.ty, types);
                let (line, arg_expr) = map_input(&pname, &pt.ty, realistic, types).ok_or_else(|| {
                    format!("`{name}` param `{pname}: {}` unsupported (v0: scalar ints, Vec<int>, Option<int>, &[int], &str/String, tuples, same-file structs/enums of these)", quote!(#pt))
                })?;
                inputs.push(line);
                args.push(arg_expr);
            }
            syn::FnArg::Receiver(_) => return Err(format!("`{name}` is a method (self)")),
        }
    }
    let call = args.join(", ");
    let tail = match postcond {
        Some(expr) => format!(
            "    let result = {name}({call});\n    kani::assert({expr}, \"aiv postcondition\");\n    let _ = &result;"
        ),
        None => format!("    let _ = {name}({call});"),
    };
    // String harnesses use the lower str_unwind() — measured necessary for strings
    // to resolve at all; a shortfall only ever yields INCONCLUSIVE, never a false pass.
    let uw = if stringy { str_unwind() } else { unwind() };
    Ok(format!(
        "#[kani::proof]\n#[kani::unwind({uw})]\nfn verify_{name}() {{\n{}\n{}\n}}",
        inputs.join("\n"),
        tail,
    ))
}

/// Recursively collect `.rs` files under `dir` in a deterministic (sorted) order.
fn collect_rs(dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p.to_string_lossy().into_owned());
        }
    }
}

fn first_fn(file: &syn::File) -> Option<&syn::ItemFn> {
    file.items.iter().find_map(|it| {
        if let syn::Item::Fn(f) = it {
            Some(f)
        } else {
            None
        }
    })
}

/// Build a lib.rs for a SINGLE function (the first one) — used by --emit / --prove.
fn build(src: &str, realistic: bool, postcond: Option<&str>) -> Result<(String, String), String> {
    let file = syn::parse_file(src).map_err(|e| format!("parse error: {e}"))?;
    let types = collect_types(&file);
    let func = first_fn(&file).ok_or("no top-level function found")?;
    let name = func.sig.ident.to_string();
    let harness = harness_for(func, realistic, postcond, &types)?;
    let lib = format!("{}\n\n{}\n\n{harness}\n", helper(), src.trim());
    Ok((name, lib))
}

/// Per-function build status: `None` = supported (harness emitted), `Some(reason)`
/// = skipped, in source order.
type FnStatuses = Vec<(String, Option<String>)>;

/// Build a lib.rs with a harness for EVERY top-level function (real source files
/// have many). Returns per-fn status plus the combined lib.
fn build_all(src: &str, realistic: bool) -> Result<(FnStatuses, String), String> {
    let file = syn::parse_file(src).map_err(|e| format!("parse error: {e}"))?;
    let funcs: Vec<&syn::ItemFn> = file
        .items
        .iter()
        .filter_map(|it| {
            if let syn::Item::Fn(f) = it {
                Some(f)
            } else {
                None
            }
        })
        .collect();
    if funcs.is_empty() {
        return Err("no top-level function found".into());
    }
    let types = collect_types(&file);
    let mut statuses = Vec::new();
    let mut harnesses = Vec::new();
    for f in &funcs {
        let name = f.sig.ident.to_string();
        match harness_for(f, realistic, None, &types) {
            Ok(h) => {
                statuses.push((name, None));
                harnesses.push(h);
            }
            Err(e) => statuses.push((name, Some(e))),
        }
    }
    let lib = format!(
        "{}\n\n{}\n\n{}\n",
        helper(),
        src.trim(),
        harnesses.join("\n\n")
    );
    Ok((statuses, lib))
}

/// Run `cargo kani` in `dir`, draining stdout+stderr on threads (so a full pipe
/// buffer can't deadlock the child) and killing it past `TIMEOUT_SECS`.
/// Returns None if the run timed out or couldn't start.
fn kani_output(dir: &PathBuf, timeout_secs: u64) -> Option<String> {
    let mut child = Command::new("cargo")
        .arg("kani")
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let mut so = child.stdout.take()?;
    let mut se = child.stderr.take()?;
    let ho = thread::spawn(move || {
        let mut s = String::new();
        so.read_to_string(&mut s).ok();
        s
    });
    let he = thread::spawn(move || {
        let mut s = String::new();
        se.read_to_string(&mut s).ok();
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > Duration::from_secs(timeout_secs) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None; // timed out — pipes hit EOF, reader threads unblock
                }
                thread::sleep(Duration::from_millis(200));
            }
            Err(_) => return None,
        }
    }
    Some(ho.join().unwrap_or_default() + &he.join().unwrap_or_default())
}

fn run_kani(
    dir: &PathBuf,
    name: &str,
    lib: &str,
    timeout_secs: u64,
) -> Option<BTreeMap<String, (bool, Vec<String>)>> {
    fs::create_dir_all(dir.join("src")).ok();
    fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"aiv_{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n"),
    )
    .ok();
    fs::write(dir.join("src/lib.rs"), lib).ok();
    let text = kani_output(dir, timeout_secs)?;
    let mut res = BTreeMap::new();
    let mut cur = String::new();
    for ln in text.lines() {
        if let Some(i) = ln.find("Checking harness verify_") {
            cur = ln[i + "Checking harness verify_".len()..]
                .split(['.', ' '])
                .next()
                .unwrap_or("")
                .to_string();
            res.insert(cur.clone(), (false, Vec::new()));
        } else if !cur.is_empty() && ln.contains("Failed Checks:") {
            if let Some(e) = res.get_mut(&cur) {
                let msg = ln.split("Failed Checks:").nth(1).unwrap_or("").trim();
                e.1.push(humanize_check(msg));
            }
        } else if !cur.is_empty() && ln.contains("VERIFICATION:-") {
            if let Some(e) = res.get_mut(&cur) {
                e.0 = ln.contains("FAILED");
            }
        }
    }
    Some(res)
}

/// Re-run a failing harness with concrete playback to extract the triggering input.
/// Returns (assertion message, symbolic input values in order).
fn counterexample(dir: &PathBuf, name: &str) -> Option<(String, Vec<String>)> {
    let out = Command::new("cargo")
        .args([
            "kani",
            "--harness",
            &format!("verify_{name}"),
            "-Z",
            "concrete-playback",
            "--concrete-playback=print",
        ])
        .current_dir(dir)
        .output()
        .ok()?;
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    parse_playback(&text)
}

/// Parse Kani `--concrete-playback=print` output into (assertion message, input
/// values in order). Pure so it can be unit-tested against captured Kani output.
///
/// Kani prints one `concrete_vals` block per failing check; each is a complete,
/// independent witness. We keep only the FIRST — merging tokens across blocks
/// produces a meaningless list (e.g. mixing a postcondition witness with a
/// separate panic witness).
fn parse_playback(text: &str) -> Option<(String, Vec<String>)> {
    let (mut assertion, mut vals, mut in_block, mut done) =
        (String::new(), Vec::new(), false, false);
    for ln in text.lines() {
        if assertion.is_empty() {
            if let Some(i) = ln.find("Check for `assertion`: \"") {
                assertion = ln[i + "Check for `assertion`: \"".len()..]
                    .trim_end_matches('"')
                    .to_string();
            }
        }
        if !done && ln.contains("let concrete_vals") {
            in_block = true;
            continue;
        }
        if in_block {
            let t = ln.trim();
            if t.starts_with("//") {
                vals.push(t.trim_start_matches("//").trim().to_string());
            }
            if t.contains("];") {
                in_block = false;
                done = true; // first block captured — ignore any further blocks
            }
        }
    }
    (!assertion.is_empty() || !vals.is_empty()).then_some((assertion, vals))
}

// Unit tests are kept next to the pure functions they cover; the CLI plumbing
// (main/batch/selftest) follows below, hence the allow.
#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{build, interpret_val_ctx, len_hint_for, map_input, parse_playback, LenHint};

    /// Test alias: interpret a token with the default (Vector) length-noun.
    fn interpret_val(tok: &str) -> String {
        interpret_val_ctx(tok, LenHint::Vector)
    }

    /// Empty type registry for the map_input tests that don't exercise structs/enums.
    fn no_types() -> super::Types {
        super::Types::default()
    }

    #[test]
    fn tuple_param_binds_one_any_per_element() {
        let (name, lib) = build("fn f(p: (i32, u8)) -> i32 { 0 }", false, None).unwrap();
        assert_eq!(name, "f");
        assert!(lib.contains("let p: (i32, u8) = (kani::any(), kani::any());"));
        assert!(lib.contains("let _ = f(p);"));
    }

    #[test]
    fn slice_param_passes_by_reference() {
        let (_, lib) = build("fn g(xs: &[i32]) -> i32 { 0 }", false, None).unwrap();
        assert!(lib.contains("let xs: Vec<i32> = any_bounded_vec::<i32>(3);"));
        assert!(lib.contains("let _ = g(&xs);"));
    }

    #[test]
    fn postcondition_binds_result_and_asserts() {
        let (_, lib) = build("fn h(a: i32) -> i32 { a }", true, Some("result >= a")).unwrap();
        assert!(lib.contains("let result = h(a);"));
        assert!(lib.contains(r#"kani::assert(result >= a, "aiv postcondition");"#));
    }

    #[test]
    fn option_payload_clamped_in_realistic_not_strict() {
        // strict: no clamp on the Some payload
        let (_, strict) = build("fn f(o: Option<i32>) -> i32 { 0 }", false, None).unwrap();
        assert!(strict.contains("let o: Option<i32> = kani::any();"));
        assert!(!strict.contains("if let Some(v) = o"));
        // realistic: clamp Some(_) so an extreme-only overflow reads as UNGUARDED, not BUG
        let (_, real) = build("fn f(o: Option<i32>) -> i32 { 0 }", true, None).unwrap();
        assert!(real.contains("if let Some(v) = o { kani::assume(v >= -1000 && v <= 1000); }"));
    }

    #[test]
    fn collect_rs_finds_rs_recursively_sorted() {
        use super::collect_rs;
        use std::fs;
        let root = std::env::temp_dir().join("aiv-collect-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("b.rs"), "").unwrap();
        fs::write(root.join("a.rs"), "").unwrap();
        fs::write(root.join("note.txt"), "").unwrap(); // ignored
        fs::write(root.join("sub").join("c.rs"), "").unwrap();
        let mut out = Vec::new();
        collect_rs(&root, &mut out);
        let names: Vec<String> = out
            .iter()
            .map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["a.rs", "b.rs", "c.rs"]); // sorted, recursive, .txt skipped
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn build_all_emits_a_harness_per_function() {
        use super::build_all;
        let src = "fn a(x: i32) -> i32 { x }\nfn b(x: f64) -> i32 { 0 }\nfn c(v: Vec<i32>) -> i32 { v[0] }";
        let (statuses, lib) = build_all(src, false).unwrap();
        let names: Vec<&str> = statuses.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        // a and c are supported → harnesses present; b (f64) is skipped with a reason
        assert!(statuses[0].1.is_none());
        assert!(statuses[1].1.is_some());
        assert!(statuses[2].1.is_none());
        assert!(lib.contains("fn verify_a()"));
        assert!(lib.contains("fn verify_c()"));
        assert!(!lib.contains("fn verify_b()"));
    }

    #[test]
    fn unsupported_param_is_rejected() {
        // f64 is outside v0 scope (no float support) — must still reject cleanly.
        assert!(map_input("x", &syn::parse_str("f64").unwrap(), false, &no_types()).is_none());
        assert!(build("fn f(x: f64) {}", false, None).is_err());
    }

    #[test]
    fn strict_only_nonoverflow_failure_is_inconclusive_not_unguarded() {
        use super::{classify, Verdict};
        use std::collections::BTreeMap;
        use std::path::PathBuf;
        let dir = PathBuf::from("/nonexistent"); // never read: only the Bug branch shells out
        // Strict fails on a non-overflow panic, realistic clean → must NOT be UNGUARDED
        // (that would claim "safe on normal input" for a real unwrap panic).
        let mut strict: BTreeMap<String, (bool, Vec<String>)> = BTreeMap::new();
        let mut real: BTreeMap<String, (bool, Vec<String>)> = BTreeMap::new();
        strict.insert(
            "f".into(),
            (true, vec!["called `Option::unwrap()` on a `None` value".into()]),
        );
        real.insert("f".into(), (false, vec![]));
        assert!(matches!(
            classify("f", &dir, &strict, &real, LenHint::Str),
            Verdict::Inconclusive
        ));
        // But a strict-only integer OVERFLOW is still legitimately UNGUARDED.
        strict.insert(
            "g".into(),
            (true, vec!["attempt to multiply with overflow".into()]),
        );
        real.insert("g".into(), (false, vec![]));
        assert!(matches!(
            classify("g", &dir, &strict, &real, LenHint::Vector),
            Verdict::Unguarded(_)
        ));
    }

    #[test]
    fn struct_param_synthesizes_field_bindings() {
        // A same-file struct is bound field-by-field, then constructed and passed.
        let src = "struct Order { qty: u64, price: u64 }\n\
                   fn charge(o: Order) -> u64 { o.qty * o.price }";
        let (name, lib) = build(src, false, None).unwrap();
        assert_eq!(name, "charge");
        assert!(lib.contains("let o_qty: u64 = kani::any();"));
        assert!(lib.contains("let o_price: u64 = kani::any();"));
        assert!(lib.contains("let o: Order = Order { qty: o_qty, price: o_price };"));
        assert!(lib.contains("let _ = charge(o);"));
    }

    #[test]
    fn struct_with_unsupported_field_is_rejected() {
        // A float field is unsupported → the whole struct bounces cleanly (never wrong).
        let src = "struct P { x: f64 }\nfn f(p: P) -> i32 { 0 }";
        assert!(build(src, false, None).is_err());
    }

    #[test]
    fn fieldless_enum_param_selects_a_variant() {
        // A C-like enum: pick a variant via a nondeterministic selector.
        let src = "enum Dir { N, S, E, W }\nfn step(d: Dir) -> i32 { 0 }";
        let (name, lib) = build(src, false, None).unwrap();
        assert_eq!(name, "step");
        assert!(lib.contains("let d_sel: usize = kani::any();"));
        assert!(lib.contains("match d_sel % 4"));
        assert!(lib.contains("0 => Dir::N,"));
        assert!(lib.contains("_ => Dir::W,")); // last variant is the catch-all
        assert!(lib.contains("let _ = step(d);"));
    }

    #[test]
    fn enum_with_data_variants_binds_fields() {
        // Tuple + struct + unit variants all synthesized.
        let src = "enum Shape { Dot, Circle(u32), Rect { w: u32, h: u32 } }\n\
                   fn area(s: Shape) -> u32 { 0 }";
        let (_, lib) = build(src, false, None).unwrap();
        assert!(lib.contains("0 => Shape::Dot,"));
        assert!(lib.contains("Shape::Circle(s_1_0)"));
        assert!(lib.contains("Shape::Rect { w: s_2_w, h: s_2_h }"));
    }

    #[test]
    fn enum_with_unsupported_field_is_rejected() {
        // A variant carrying a float bounces the whole enum (never a wrong answer).
        let src = "enum E { A(f64), B }\nfn f(e: E) -> i32 { 0 }";
        assert!(build(src, false, None).is_err());
    }

    #[test]
    fn nested_struct_recurses() {
        // A struct field that is itself a same-file struct is synthesized recursively.
        let src = "struct Inner { a: i32 }\n\
                   struct Outer { inner: Inner, b: u8 }\n\
                   fn f(o: Outer) -> i32 { o.inner.a }";
        let (_, lib) = build(src, false, None).unwrap();
        assert!(lib.contains("let o_inner_a: i32 = kani::any();"));
        assert!(lib.contains("let o_inner: Inner = Inner { a: o_inner_a };"));
        assert!(lib.contains("let o_b: u8 = kani::any();"));
        assert!(lib.contains("let o: Outer = Outer { inner: o_inner, b: o_b };"));
    }

    #[test]
    fn str_and_string_params_supported() {
        // &str binds a bounded symbolic ASCII String (its OWN default bound of 1,
        // decoupled from the Vec --bound of 3), passed by reference (coerces to &str).
        let (_, lib) = build("fn f(s: &str) -> usize { s.len() }", false, None).unwrap();
        assert!(lib.contains("let s: String = any_bounded_string(1);"));
        assert!(lib.contains("let _ = f(&s);"));
        // A string harness uses the lower str_unwind (2), not the default unwind (5).
        assert!(lib.contains("#[kani::unwind(2)]"));
        // Owned String binds the same symbolic value, passed by move.
        let (_, lib2) = build("fn g(s: String) -> usize { s.len() }", false, None).unwrap();
        assert!(lib2.contains("let s: String = any_bounded_string(1);"));
        assert!(lib2.contains("let _ = g(s);"));
        // A non-string harness keeps the default unwind (5) — decoupling is scoped.
        let (_, num) = build("fn n(x: i32) -> i32 { x }", false, None).unwrap();
        assert!(num.contains("#[kani::unwind(5)]"));
    }

    #[test]
    fn json_escapes_special_chars() {
        use super::json_escape;
        assert_eq!(json_escape(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(json_escape("line\nbreak"), "line\\nbreak");
    }

    #[test]
    fn verdict_json_shapes() {
        use super::{verdict_json, Verdict};
        let bug = Verdict::Bug(vec!["divide by zero".into()], vec!["0".into()]);
        assert_eq!(
            verdict_json("swap_div", &bug),
            r#"{"name":"swap_div","verdict":"BUG","checks":["divide by zero"],"witness":["0"]}"#
        );
        assert_eq!(
            verdict_json("ok", &Verdict::Verified),
            r#"{"name":"ok","verdict":"VERIFIED","checks":[],"witness":[]}"#
        );
    }

    // Two failing checks → two concrete_vals blocks. We must keep only the first
    // witness, not merge tokens across blocks (the find_max --prove regression).
    #[test]
    fn playback_keeps_only_first_witness_block() {
        let kani = r#"
    let concrete_vals: Vec<Vec<u8>> = vec![
        // 1ul
        vec![1, 0, 0, 0, 0, 0, 0, 0],
        // -1
        vec![255, 255, 255, 255],
    ];
    kani::concrete_playback_run(concrete_vals, verify_find_max);
    let concrete_vals: Vec<Vec<u8>> = vec![
        // 0ul
        vec![0, 0, 0, 0, 0, 0, 0, 0],
    ];
"#;
        let (_, vals) = parse_playback(kani).unwrap();
        assert_eq!(vals, vec!["1ul", "-1"]); // NOT ["1ul","-1","0ul"]
    }

    // One failing check with two params → one block holding all param values.
    // The first-block rule must NOT truncate this (the dot_index case).
    #[test]
    fn playback_keeps_all_params_in_one_block() {
        let kani = r#"
    let concrete_vals: Vec<Vec<u8>> = vec![
        // 1ul
        vec![1, 0, 0, 0, 0, 0, 0, 0],
        // -1
        vec![255, 255, 255, 255],
        // 0ul
        vec![0, 0, 0, 0, 0, 0, 0, 0],
    ];
"#;
        let (_, vals) = parse_playback(kani).unwrap();
        assert_eq!(vals, vec!["1ul", "-1", "0ul"]);
    }

    #[test]
    fn playback_captures_assertion_message() {
        let kani = "Check for `assertion`: \"attempt to add with overflow\"\n    let concrete_vals: Vec<Vec<u8>> = vec![\n        // 5i32\n    ];";
        let (msg, vals) = parse_playback(kani).unwrap();
        assert_eq!(msg, "attempt to add with overflow");
        assert_eq!(vals, vec!["5i32"]);
    }

    #[test]
    fn playback_none_when_no_block() {
        assert!(parse_playback("VERIFICATION:- SUCCESSFUL\n").is_none());
    }

    #[test]
    fn fail_on_levels_gate_correctly() {
        use super::{fails, Verdict, FAIL_ON};
        use std::sync::atomic::Ordering;
        let bug = Verdict::Bug(vec![], vec![]);
        let ung = Verdict::Unguarded(vec![]);
        let inc = Verdict::Inconclusive;
        let ver = Verdict::Verified;
        FAIL_ON.store(0, Ordering::Relaxed); // bug only (default)
        assert!(fails(&bug) && !fails(&ung) && !fails(&inc) && !fails(&ver));
        FAIL_ON.store(1, Ordering::Relaxed); // + unguarded
        assert!(fails(&bug) && fails(&ung) && !fails(&inc) && !fails(&ver));
        FAIL_ON.store(2, Ordering::Relaxed); // + inconclusive
        assert!(fails(&bug) && fails(&ung) && fails(&inc) && !fails(&ver));
        FAIL_ON.store(0, Ordering::Relaxed); // reset for other tests
    }

    #[test]
    fn humanize_check_rewrites_kani_placeholder() {
        use super::humanize_check;
        // Kani's runtime-format placeholder becomes a readable panic description.
        let placeholder = "This is a placeholder message; Kani doesn't support message formatted at runtime";
        assert!(humanize_check(placeholder).contains("reachable panic"));
        assert!(!humanize_check(placeholder).contains("placeholder"));
        // Any other check text is passed through verbatim (trimmed).
        assert_eq!(
            humanize_check("  attempt to divide by zero  "),
            "attempt to divide by zero"
        );
        // Must NOT touch the unwinding-assertion marker real_failures() filters on.
        assert_eq!(
            humanize_check("unwinding assertion loop 0"),
            "unwinding assertion loop 0"
        );
    }

    #[test]
    fn unwinding_assertions_are_not_real_failures() {
        use super::real_failures;
        // a loop that overran the unwind bound is NOT a bug — filtered out
        let only_unwind = vec!["unwinding assertion loop 0".to_string()];
        assert!(real_failures(&only_unwind).is_empty());
        // a genuine panic is kept
        let mixed = vec![
            "unwinding assertion loop 0".to_string(),
            "attempt to multiply with overflow".to_string(),
        ];
        assert_eq!(
            real_failures(&mixed),
            vec!["attempt to multiply with overflow"]
        );
    }

    #[test]
    fn usize_reads_as_vector_length() {
        assert_eq!(interpret_val("0ul"), "0 (usize → empty vector)");
        assert_eq!(interpret_val("1ul"), "1 (usize → 1-element vector)");
        assert_eq!(interpret_val("3ul"), "3 (usize → vector of length 3)");
    }
    #[test]
    fn implausibly_large_usize_is_not_a_vector_length() {
        // An enum selector / unconstrained usize far exceeds the bound (3) — must NOT
        // be printed as a giant vector; show the raw number instead.
        assert_eq!(interpret_val("9223372036854775807ul"), "9223372036854775807");
        assert_eq!(interpret_val("500ul"), "500"); // 500 > bound(3) → plain
    }
    #[test]
    fn string_hint_renders_string_noun_not_vector() {
        // The exact gap this fixes: a usize length in a string function's witness
        // must read "string", not "vector".
        assert_eq!(
            interpret_val_ctx("0ul", LenHint::Str),
            "0 (usize → empty string)"
        );
        assert_eq!(
            interpret_val_ctx("1ul", LenHint::Str),
            "1 (usize → 1-char string)"
        );
        // Vector hint keeps the original noun (unchanged behavior / RESULTS.md).
        assert_eq!(interpret_val_ctx("0ul", LenHint::Vector), "0 (usize → empty vector)");
        assert_eq!(interpret_val("0ul"), "0 (usize → empty vector)"); // default = Vector
        // A function with both a string and a vec can't be disambiguated → neutral.
        assert_eq!(
            interpret_val_ctx("0ul", LenHint::Ambiguous),
            "0 (usize → empty collection)"
        );
        // Non-usize tokens are unaffected by the hint (sentinels still name themselves).
        assert_eq!(interpret_val_ctx("255u8", LenHint::Str), "255 (u8::MAX)");
    }

    #[test]
    fn len_hint_reads_param_types() {
        let str_only: syn::ItemFn = syn::parse_str("fn f(s: &str) -> usize { 0 }").unwrap();
        assert!(matches!(len_hint_for(&str_only), LenHint::Str));
        let string_owned: syn::ItemFn =
            syn::parse_str("fn f(s: String) -> usize { 0 }").unwrap();
        assert!(matches!(len_hint_for(&string_owned), LenHint::Str));
        let vec_only: syn::ItemFn = syn::parse_str("fn f(v: Vec<i32>) -> i32 { 0 }").unwrap();
        assert!(matches!(len_hint_for(&vec_only), LenHint::Vector));
        let slice_only: syn::ItemFn = syn::parse_str("fn f(v: &[i32]) -> i32 { 0 }").unwrap();
        assert!(matches!(len_hint_for(&slice_only), LenHint::Vector));
        let both: syn::ItemFn =
            syn::parse_str("fn f(s: String, v: &[i32]) -> i32 { 0 }").unwrap();
        assert!(matches!(len_hint_for(&both), LenHint::Ambiguous));
        // No collections at all → Vector default (no usize tokens will appear anyway).
        let scalars: syn::ItemFn = syn::parse_str("fn f(a: i32, b: u8) -> i32 { 0 }").unwrap();
        assert!(matches!(len_hint_for(&scalars), LenHint::Vector));
    }

    #[test]
    fn names_overflow_sentinels() {
        assert_eq!(interpret_val("-2147483648i32"), "-2147483648 (i32::MIN)");
        assert_eq!(interpret_val("2147483647i32"), "2147483647 (i32::MAX)");
        assert_eq!(interpret_val("127i8"), "127 (i8::MAX)");
        assert_eq!(interpret_val("255u8"), "255 (u8::MAX)");
    }
    #[test]
    fn ordinary_values_pass_through_without_suffix() {
        assert_eq!(interpret_val("5i32"), "5");
        assert_eq!(interpret_val("-3i64"), "-3");
        assert_eq!(interpret_val("42"), "42");
        assert_eq!(interpret_val("true"), "true");
        // a non-extreme value that shares a width but isn't the sentinel
        assert_eq!(interpret_val("100i8"), "100");
    }
}

/// Turn a raw Kani concrete value token (`"0ul"`, `"-2147483648i32"`, `"true"`)
/// into something a human reads at a glance. Conservative: only annotates values
/// it can identify unambiguously (min/max sentinels, and — via `hint` — a `usize`
/// collection length as vector vs string); otherwise strips the type suffix and
/// passes the number through.
fn interpret_val_ctx(tok: &str, hint: LenHint) -> String {
    let t = tok.trim();
    if t == "true" || t == "false" {
        return t.to_string();
    }
    // split "-123" core from "i32"/"ul"/... suffix
    let split = t.find(|c: char| c.is_ascii_alphabetic() && c != '-');
    let (core, suffix) = match split {
        Some(i) => (&t[..i], &t[i..]),
        None => (t, ""),
    };
    // A usize token is a symbolic collection length — Vec/slice or string, per hint.
    if suffix == "ul" || suffix == "usize" {
        // A synthesized length is always ≤ the bound (`kani::assume(len <= bound)`).
        // A larger usize can't be one of our lengths — it's an enum variant selector
        // or an unconstrained usize param — so show it plainly, never as a giant
        // "vector of length 9223372036854775807".
        let max_len = match hint {
            LenHint::Str => str_bound(),
            LenHint::Ambiguous => bound().max(str_bound()),
            LenHint::Vector => bound(),
        };
        if core.parse::<u128>().is_ok_and(|v| v > max_len as u128) {
            return core.to_string();
        }
        return match hint {
            LenHint::Str => match core {
                "0" => "0 (usize → empty string)".to_string(),
                "1" => "1 (usize → 1-char string)".to_string(),
                n => format!("{n} (usize → string of length {n})"),
            },
            LenHint::Ambiguous => match core {
                "0" => "0 (usize → empty collection)".to_string(),
                n => format!("{n} (usize → length {n})"),
            },
            LenHint::Vector => match core {
                "0" => "0 (usize → empty vector)".to_string(),
                "1" => "1 (usize → 1-element vector)".to_string(),
                n => format!("{n} (usize → vector of length {n})"),
            },
        };
    }
    // name overflow sentinels (min/max for the signed/unsigned width)
    let sentinel = match (suffix, core) {
        ("i8", "-128") => Some("i8::MIN"),
        ("i8", "127") => Some("i8::MAX"),
        ("i16", "-32768") => Some("i16::MIN"),
        ("i16", "32767") => Some("i16::MAX"),
        ("i32", "-2147483648") => Some("i32::MIN"),
        ("i32", "2147483647") => Some("i32::MAX"),
        ("i64" | "isize", "-9223372036854775808") => Some("i64::MIN"),
        ("i64" | "isize", "9223372036854775807") => Some("i64::MAX"),
        ("u8", "255") => Some("u8::MAX"),
        ("u16", "65535") => Some("u16::MAX"),
        ("u32", "4294967295") => Some("u32::MAX"),
        _ => None,
    };
    match sentinel {
        Some(name) => format!("{core} ({name})"),
        None => core.to_string(),
    }
}

enum Verdict {
    Verified,
    Bug(Vec<String>, Vec<String>), // failed checks, human-readable witness values
    Unguarded(Vec<String>),        // overflow only at extremes
    Inconclusive,
    Unsupported(String),
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

/// One result as a JSON object. `witness` values are the human-readable form.
fn verdict_json(label: &str, v: &Verdict) -> String {
    let (kind, checks, witness): (&str, Vec<String>, Vec<String>) = match v {
        Verdict::Verified => ("VERIFIED", vec![], vec![]),
        // Witness values are already human-readable (interpreted in `classify`).
        Verdict::Bug(c, w) => ("BUG", c.clone(), w.clone()),
        Verdict::Unguarded(c) => ("UNGUARDED", c.clone(), vec![]),
        Verdict::Inconclusive => ("INCONCLUSIVE", vec![], vec![]),
        Verdict::Unsupported(e) => ("UNSUPPORTED", vec![e.clone()], vec![]),
    };
    let arr = |xs: &[String]| {
        xs.iter()
            .map(|x| format!("\"{}\"", json_escape(x)))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{{\"name\":\"{}\",\"verdict\":\"{}\",\"checks\":[{}],\"witness\":[{}]}}",
        json_escape(label),
        kind,
        arr(&checks),
        arr(&witness)
    )
}

/// Emit `{"results":[...],"summary":{...}}` for a set of labelled verdicts.
fn emit_json(results: &[(&str, &Verdict)]) {
    let count = |p: fn(&Verdict) -> bool| results.iter().filter(|(_, v)| p(v)).count();
    let bugs = count(|v| matches!(v, Verdict::Bug(..)));
    let ung = count(|v| matches!(v, Verdict::Unguarded(_)));
    let ver = count(|v| matches!(v, Verdict::Verified));
    let other = results.len() - bugs - ung - ver;
    let items: Vec<String> = results.iter().map(|(l, v)| verdict_json(l, v)).collect();
    println!(
        "{{\"results\":[{}],\"summary\":{{\"bug\":{bugs},\"unguarded\":{ung},\"verified\":{ver},\"other\":{other}}}}}",
        items.join(",")
    );
}

/// Rewrite Kani's opaque check descriptions into something a developer reads at a
/// glance. Kani emits a fixed placeholder for any panic whose message is built at
/// runtime (the common `.unwrap()`/`.expect()` on `Err`/`None`, or `panic!("{}", x)`),
/// because it can't evaluate the format string statically. Left raw, the BUG line
/// reads "This is a placeholder message; Kani doesn't support message formatted at
/// runtime" — accurate but useless. Any other check text passes through unchanged.
fn humanize_check(msg: &str) -> String {
    let m = msg.trim();
    if m.contains("placeholder message") || m.contains("message formatted at runtime") {
        return "reachable panic — unwrap/expect on Err/None, or a runtime-formatted panic!()"
            .to_string();
    }
    m.to_string()
}

/// Keep only genuine panic/overflow checks. An "unwinding assertion" failure is NOT
/// a panic — it means a loop wasn't fully unrolled within the unwind bound, so the
/// check is incomplete. Unwinding-only failures must classify as INCONCLUSIVE, never
/// BUG/UNGUARDED (that would be a false verdict — the very thing this tool prevents).
fn real_failures(checks: &[String]) -> Vec<String> {
    checks
        .iter()
        .filter(|c| !c.contains("unwinding assertion"))
        .cloned()
        .collect()
}

type KaniMap = BTreeMap<String, (bool, Vec<String>)>;

/// Dual-mode classify for one function from its strict+realistic Kani results.
/// `hint` resolves `usize` witness tokens to the right noun (vector vs string).
fn classify(
    name: &str,
    realistic_dir: &PathBuf,
    strict: &KaniMap,
    real: &KaniMap,
    hint: LenHint,
) -> Verdict {
    let s = strict.get(name).cloned().unwrap_or((false, vec![]));
    let r = real.get(name).cloned().unwrap_or((false, vec![]));
    let s_real = real_failures(&s.1);
    let r_real = real_failures(&r.1);
    if !s.0 {
        Verdict::Verified
    } else if s_real.is_empty() {
        // strict failed only on unwinding assertions — couldn't fully check the loop
        Verdict::Inconclusive
    } else if !r_real.is_empty() {
        // Interpret the witness NOW, while we know the param types via `hint`.
        let vals = counterexample(realistic_dir, name)
            .map(|(_, v)| v.iter().map(|t| interpret_val_ctx(t, hint)).collect())
            .unwrap_or_default();
        Verdict::Bug(r_real, vals)
    } else if s_real.iter().all(|c| c.contains("with overflow")) {
        // Strict-only failures that are ALL integer overflows → genuinely
        // "overflows only at the extremes". This is what UNGUARDED means.
        Verdict::Unguarded(s_real)
    } else {
        // Strict failed on a NON-overflow panic (unwrap/divide-by-zero/…) that
        // realistic didn't reproduce. Since strict and realistic bound strings and
        // collections identically, such a split isn't an "extreme-value" story — it's
        // an unstable result (typically BMC nondeterminism at a low string unwind).
        // Calling it UNGUARDED would falsely imply "safe on normal input", so report
        // INCONCLUSIVE instead — never claim safety we can't stand behind.
        Verdict::Inconclusive
    }
}

/// Verify EVERY top-level function in a source file (real files have many). Returns
/// (name, verdict) per function in source order. `slot` + PID namespace the temp dir
/// so concurrent workers never collide.
fn verify_file(src: &str, slot: usize) -> Vec<(String, Verdict)> {
    let (statuses, strict_lib) = match build_all(src, false) {
        Ok(v) => v,
        Err(e) => return vec![(String::new(), Verdict::Unsupported(e))],
    };
    let real_lib = build_all(src, true).unwrap().1;
    let base = std::env::temp_dir().join(format!("cargo-aiv-{}-multi-{slot}", std::process::id()));
    let realistic_dir = base.join("realistic");
    // All harnesses share one Kani invocation, so a file with N functions needs a
    // bigger budget than a single one — else one slow function (Kani is slow on some
    // iterator patterns) times out the whole file and every verdict is lost. Scale by
    // the supported-fn count, capped so CI stays bounded.
    let n = statuses.iter().filter(|(_, s)| s.is_none()).count().max(1) as u64;
    let timeout = (TIMEOUT_SECS * n).min(TIMEOUT_MAX_SECS);
    let (strict, real) = match (
        run_kani(&base.join("strict"), "multi", &strict_lib, timeout),
        run_kani(&realistic_dir, "multi", &real_lib, timeout),
    ) {
        (Some(s), Some(r)) => (s, r),
        // whole-file run timed out: supported fns are inconclusive, unsupported stay so
        _ => {
            return statuses
                .into_iter()
                .map(|(n, st)| match st {
                    Some(e) => (n, Verdict::Unsupported(e)),
                    None => (n, Verdict::Inconclusive),
                })
                .collect()
        }
    };
    let hints = hints_for_file(src);
    statuses
        .into_iter()
        .map(|(name, st)| match st {
            Some(e) => (name, Verdict::Unsupported(e)),
            None => {
                let hint = hints.get(&name).copied().unwrap_or(LenHint::Vector);
                let v = classify(&name, &realistic_dir, &strict, &real, hint);
                (name, v)
            }
        })
        .collect()
}

/// Exit code convention shared by single-file and batch: BUG → 1, else 0
/// (INCONCLUSIVE stays 2 in single-file for back-compat).
fn verdict_code(v: &Verdict) -> i32 {
    match v {
        Verdict::Bug(..) => 1,
        Verdict::Inconclusive => 2,
        Verdict::Unsupported(_) => 1,
        _ => 0,
    }
}

/// Detailed single-file report. Returns the process exit code.
fn print_detailed(name: &str, v: &Verdict) -> i32 {
    println!();
    match v {
        Verdict::Verified => {
            println!(
                "{G}{B}✅ VERIFIED{X}  `{name}` — panic-free for Vec≤{}, |val|≤{RANGE}.",
                bound()
            );
        }
        Verdict::Bug(checks, vals) => {
            println!("{R}{B}🔴 BUG{X}  `{name}` — panic reachable on ordinary input:");
            for c in checks {
                println!("     {R}• {c}{X}");
            }
            if !vals.is_empty() {
                // vals are already human-readable (interpreted in `classify`).
                println!(
                    "     {DIM}reachable with input(s), in order: {}{X}",
                    vals.join(", ")
                );
            }
        }
        Verdict::Unguarded(checks) => {
            println!("{Y}{B}🟡 UNGUARDED{X}  `{name}` — safe on normal input, but overflows at i32::MAX/MIN:");
            for c in checks {
                println!("     {Y}• {c}{X}");
            }
            println!("     {DIM}add a bounds guard or use checked_/saturating_ arithmetic.{X}");
        }
        Verdict::Inconclusive => {
            println!(
                "{Y}{B}⏱️  INCONCLUSIVE{X}  `{name}` — no stable verdict at the current bounds."
            );
            println!("     {DIM}hit the {TIMEOUT_SECS}s/mode timeout, or gave an unstable strict/realistic split (common for strings at a low unwind). Raise --bound/--unwind/--str-unwind. Not a pass — not a bug.{X}");
        }
        Verdict::Unsupported(e) => {
            println!("{Y}⏭  cargo-aiv: {e}{X}");
        }
    }
    verdict_code(v)
}

/// Batch mode: verify every file in parallel (bounded workers), print a compact
/// table in input order, exit 1 if any BUG. Parallelism is what makes verifying a
/// whole crate tractable — sequential Kani runs over N functions don't scale.
fn batch(files: &[String], json: bool) -> i32 {
    let tag = |v: &Verdict| match v {
        Verdict::Verified => format!("{G}✅ VERIFIED{X}"),
        Verdict::Bug(..) => format!("{R}🔴 BUG{X}"),
        Verdict::Unguarded(_) => format!("{Y}🟡 UNGUARDED{X}"),
        Verdict::Inconclusive => format!("{Y}⏱️  INCONCLUSIVE{X}"),
        Verdict::Unsupported(_) => format!("{DIM}⏭  unsupported{X}"),
    };
    // Kani/CBMC is CPU- and memory-heavy and already multi-threaded, so each run
    // wants most of the machine. Over-parallelizing starves individual runs past
    // their timeout → FALSE ⏱️ INCONCLUSIVE. Keep concurrency low: ~cores/4, ≤3.
    let workers = std::thread::available_parallelism()
        .map(|n| (n.get() / 4).max(1))
        .unwrap_or(2)
        .clamp(1, 3)
        .min(files.len().max(1));
    if !json {
        println!(
            "{DIM}cargo-aiv batch — {} file(s), {workers} workers, ≤{TIMEOUT_SECS}s/mode each{X}\n",
            files.len()
        );
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: std::sync::Mutex<Vec<(usize, String, Verdict)>> =
        std::sync::Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= files.len() {
                    break;
                }
                let f = &files[i];
                match fs::read_to_string(f) {
                    Ok(src) => {
                        // one file can hold many functions — record a row per function
                        for (name, v) in verify_file(&src, i) {
                            let label = if name.is_empty() { f.clone() } else { name };
                            results.lock().unwrap().push((i, label, v));
                        }
                    }
                    Err(e) => results.lock().unwrap().push((
                        i,
                        f.clone(),
                        Verdict::Unsupported(e.to_string()),
                    )),
                }
            });
        }
    });
    let mut out = results.into_inner().unwrap();
    out.sort_by_key(|(i, _, _)| *i);
    let (mut bugs, mut ung, mut ver, mut other) = (0, 0, 0, 0);
    for (_, _, v) in &out {
        match v {
            Verdict::Bug(..) => bugs += 1,
            Verdict::Unguarded(_) => ung += 1,
            Verdict::Verified => ver += 1,
            _ => other += 1,
        }
    }
    let exit = i32::from(out.iter().any(|(_, _, v)| fails(v)));
    if json {
        let refs: Vec<(&str, &Verdict)> = out.iter().map(|(_, l, v)| (l.as_str(), v)).collect();
        emit_json(&refs);
        return exit;
    }
    for (_, label, v) in &out {
        let detail = match v {
            Verdict::Bug(_, vals) if !vals.is_empty() => {
                // vals are already human-readable (interpreted in `classify`).
                format!("  {DIM}← {}{X}", vals.join(", "))
            }
            _ => String::new(),
        };
        println!("  {:<22} {}{}", label, tag(v), detail);
    }
    println!(
        "\n{DIM}────{X}\n{R}{bugs} BUG{X} · {Y}{ung} UNGUARDED{X} · {G}{ver} VERIFIED{X} · {other} other"
    );
    exit
}

fn verdict_kind(v: &Verdict) -> &'static str {
    match v {
        Verdict::Verified => "VERIFIED",
        Verdict::Bug(..) => "BUG",
        Verdict::Unguarded(_) => "UNGUARDED",
        Verdict::Inconclusive => "INCONCLUSIVE",
        Verdict::Unsupported(_) => "UNSUPPORTED",
    }
}

/// Sanity-check the Kani wiring in the user's environment: a verifier that silently
/// passes everything (Kani missing / output format drifted) is worse than useless.
/// Run a function that MUST be BUG and one that MUST be VERIFIED; fail loudly if the
/// tool can't tell them apart.
fn selftest() -> i32 {
    println!("{DIM}cargo-aiv self-test — checking the Kani wiring in this environment…{X}");
    let one = |src: &str, slot| verify_file(src, slot).pop().map(|(_, v)| v).unwrap();
    let bug = one("fn aiv_bug(v: Vec<i32>) -> i32 { v[0] }", 900);
    let safe = one("fn aiv_safe(x: i32) -> i32 { x }", 901);
    let bug_ok = matches!(bug, Verdict::Bug(..));
    let safe_ok = matches!(safe, Verdict::Verified);
    let mark = |ok: bool| {
        if ok {
            format!("{G}OK{X}")
        } else {
            format!("{R}WRONG{X}")
        }
    };
    println!(
        "  known-bug  `v[0]` on empty vec → {:<12} expect BUG       {}",
        verdict_kind(&bug),
        mark(bug_ok)
    );
    println!(
        "  known-safe identity fn         → {:<12} expect VERIFIED  {}",
        verdict_kind(&safe),
        mark(safe_ok)
    );
    if bug_ok && safe_ok {
        println!("{G}{B}PASS{X} — Kani is wired correctly; verdicts are trustworthy.");
        0
    } else {
        println!(
            "{R}{B}FAIL{X} — the tool is NOT distinguishing known cases; DO NOT trust its results."
        );
        println!("     {DIM}Is Kani installed and set up?  cargo install --locked kani-verifier && cargo kani setup{X}");
        1
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("aiv") {
        args.remove(0); // invoked as `cargo aiv ...`
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("cargo-aiv {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--selftest") {
        std::process::exit(selftest());
    }
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print!(
            "\
cargo-aiv {ver} — prove AI-generated Rust is panic-free (don't just test it)

USAGE:
    cargo-aiv <file.rs>...             verify one file, or many (batch table)
    cargo-aiv --prove '<expr>' <file>  prove a postcondition over `result`/inputs
    cargo-aiv --emit <file.rs>         print the generated Kani harness, don't run
    cargo-aiv --json <file.rs>...      machine-readable results (single or batch)
    cargo-aiv --selftest               check Kani is wired correctly (trust guard)

OPTIONS:
    --bound N          max Vec/slice length to check (default 3)
    --str-bound N      max &str/String length to check (default 1 — strings are costly)
    --unwind N         loop unroll depth (default 5)
    --str-unwind N     loop unroll depth for string harnesses (default 2 — strings are costly)
    --fail-on <level>  which verdicts exit non-zero: bug (default) | unguarded | inconclusive

VERDICTS:
    🔴 BUG          panic reachable on ordinary input (exit 1) — with a witness
    🟡 UNGUARDED    overflows only at i32::MAX/MIN — add a guard (exit 0)
    ✅ VERIFIED     provably panic-free within bounds (exit 0)
    ⏱️  INCONCLUSIVE didn't finish within {t}s/mode (exit 2)
    ⏭  unsupported  a type outside v0 scope — skipped cleanly

Supported params: scalar ints, Vec<int>, Option<int>, &[int]/&mut [int], &str/String,
                  (int, int), and same-file structs whose fields are all of the above.
Requires Kani: cargo install --locked kani-verifier && cargo kani setup
Docs: https://github.com/ss1738/cargo-aiv
",
            ver = env!("CARGO_PKG_VERSION"),
            t = TIMEOUT_SECS
        );
        return;
    }
    let emit = args.iter().any(|a| a == "--emit");
    let json = args.iter().any(|a| a == "--json");
    // --prove '<expr>' consumes the following arg as the postcondition
    let mut prove: Option<String> = None;
    if let Some(i) = args.iter().position(|a| a == "--prove") {
        match args.get(i + 1).cloned() {
            Some(expr) => {
                prove = Some(expr);
                args.drain(i..=i + 1);
            }
            None => {
                eprintln!("usage: cargo-aiv --prove '<bool expr over result/inputs>' <file.rs>");
                std::process::exit(2);
            }
        }
    }
    // --bound N / --unwind N override the BMC depth. Drain the value too so it's
    // not mistaken for a file path.
    if let Some(i) = args.iter().position(|a| a == "--bound") {
        match args.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
            Some(v) if v >= 1 => {
                BOUND_A.store(v, Ordering::Relaxed);
                args.drain(i..=i + 1);
            }
            _ => {
                eprintln!("--bound needs a positive integer");
                std::process::exit(2);
            }
        }
    }
    // --str-bound N overrides the (decoupled, low-by-default) symbolic string length.
    // Allows 0 (empty string only — the fastest, highest-value case).
    if let Some(i) = args.iter().position(|a| a == "--str-bound") {
        match args.get(i + 1).and_then(|s| s.parse::<usize>().ok()) {
            Some(v) => {
                STR_BOUND_A.store(v, Ordering::Relaxed);
                args.drain(i..=i + 1);
            }
            _ => {
                eprintln!("--str-bound needs a non-negative integer");
                std::process::exit(2);
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--unwind") {
        match args.get(i + 1).and_then(|s| s.parse::<u32>().ok()) {
            Some(v) if v >= 1 => {
                UNWIND_A.store(v, Ordering::Relaxed);
                args.drain(i..=i + 1);
            }
            _ => {
                eprintln!("--unwind needs a positive integer");
                std::process::exit(2);
            }
        }
    }
    // --str-unwind N overrides the (decoupled, low-by-default) unwind for harnesses
    // with a string param — raise it if a string function has real internal loops.
    if let Some(i) = args.iter().position(|a| a == "--str-unwind") {
        match args.get(i + 1).and_then(|s| s.parse::<u32>().ok()) {
            Some(v) if v >= 1 => {
                STR_UNWIND_A.store(v, Ordering::Relaxed);
                args.drain(i..=i + 1);
            }
            _ => {
                eprintln!("--str-unwind needs a positive integer");
                std::process::exit(2);
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--fail-on") {
        let level = match args.get(i + 1).map(|s| s.as_str()) {
            Some("bug") => 0,
            Some("unguarded") => 1,
            Some("inconclusive") => 2,
            _ => {
                eprintln!("--fail-on needs one of: bug | unguarded | inconclusive");
                std::process::exit(2);
            }
        };
        FAIL_ON.store(level, Ordering::Relaxed);
        args.drain(i..=i + 1);
    }
    let raw: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    if raw.is_empty() {
        eprintln!("usage: cargo-aiv [--emit] [--prove '<expr>'] <file.rs | dir>...");
        std::process::exit(2);
    }
    // A directory argument expands to every .rs file under it (recursive) — so
    // `cargo-aiv src/` verifies a whole crate without a shell glob.
    let mut files = Vec::new();
    for a in &raw {
        let p = std::path::Path::new(a);
        if p.is_dir() {
            collect_rs(p, &mut files);
        } else {
            files.push(a.clone());
        }
    }
    if files.is_empty() {
        eprintln!("cargo-aiv: no .rs files found in {}", raw.join(", "));
        std::process::exit(2);
    }

    // Batch mode: 2+ files → summary table, one exit code (only with plain verify).
    if files.len() > 1 && !emit && prove.is_none() {
        std::process::exit(batch(&files, json));
    }

    let file = files[0].clone();
    let src = fs::read_to_string(&file).unwrap_or_else(|e| {
        eprintln!("cannot read {file}: {e}");
        std::process::exit(2);
    });

    if emit {
        match build(&src, prove.is_some(), prove.as_deref()) {
            Ok((_, lib)) => println!("{lib}"),
            Err(e) => {
                eprintln!("cargo-aiv: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // --prove: check a postcondition over the return value (realistic bounds).
    if let Some(expr) = prove {
        let (name, lib) = match build(&src, true, Some(&expr)) {
            Ok(v) => v,
            Err(e) => {
                println!("{Y}⏭  cargo-aiv: {e}{X}");
                std::process::exit(1);
            }
        };
        println!(
            "{DIM}proving `{name}`: {B}{expr}{X}{DIM}  (all inputs, |val|≤{RANGE}, Vec≤{})…{X}",
            bound()
        );
        let dir =
            std::env::temp_dir().join(format!("cargo-aiv-{}-{name}-prove", std::process::id()));
        let res = match run_kani(&dir, &name, &lib, TIMEOUT_SECS) {
            Some(r) => r,
            None => {
                println!(
                    "\n{Y}{B}⏱️  INCONCLUSIVE{X}  `{name}` — didn't finish within {TIMEOUT_SECS}s."
                );
                std::process::exit(2);
            }
        };
        let v = res.get(&name).cloned().unwrap_or((false, vec![]));
        println!();
        if !v.0 {
            println!("{G}{B}✅ PROVEN{X}  `{name}` — {B}{expr}{X} holds for all inputs in bounds.");
            std::process::exit(0);
        }
        println!(
            "{R}{B}🔴 VIOLATED{X}  `{name}` — {B}{expr}{X} can be false (or the fn panics first):"
        );
        for c in &v.1 {
            println!("     {R}• {c}{X}");
        }
        if let Some((_a, vals)) = counterexample(&dir, &name) {
            if !vals.is_empty() {
                let hint = hints_for_file(&src)
                    .get(&name)
                    .copied()
                    .unwrap_or(LenHint::Vector);
                let pretty: Vec<String> = vals.iter().map(|v| interpret_val_ctx(v, hint)).collect();
                println!(
                    "     {DIM}counterexample input(s), in order: {}{X}",
                    pretty.join(", ")
                );
            }
        }
        std::process::exit(1);
    }

    // Single-file verify: every top-level function, dual-mode classify.
    if !json {
        println!("{DIM}verifying `{file}` (strict + realistic bounded model checking, ≤{TIMEOUT_SECS}s/mode)…{X}");
    }
    let results = verify_file(&src, 0);
    let label = |n: &str| {
        if n.is_empty() {
            file.clone()
        } else {
            n.to_string()
        }
    };
    // exit 1 if any verdict fails the gate; else a lone INCONCLUSIVE keeps its
    // distinct exit 2 (couldn't check ≠ verified).
    let single_inconclusive = results.len() == 1 && matches!(results[0].1, Verdict::Inconclusive);
    let exit = if results.iter().any(|(_, v)| fails(v)) {
        1
    } else if single_inconclusive {
        2
    } else {
        0
    };
    if json {
        let labels: Vec<(String, &Verdict)> = results.iter().map(|(n, v)| (label(n), v)).collect();
        let refs: Vec<(&str, &Verdict)> = labels.iter().map(|(l, v)| (l.as_str(), *v)).collect();
        emit_json(&refs);
        std::process::exit(exit);
    }
    for (name, verdict) in &results {
        print_detailed(&label(name), verdict);
    }
    std::process::exit(exit);
}
