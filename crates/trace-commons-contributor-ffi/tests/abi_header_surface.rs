//! Holds the two hand-maintained copies of the C ABI header to the Rust
//! `extern "C"` surface they describe.
//!
//! There are two copies of `trace_commons.h`: this crate's
//! `include/trace_commons.h`, and `macos/Sources/CTraceCommons/include/
//! trace_commons.h`, which the Swift package's `CTraceCommons`
//! `systemLibrary` target names as its umbrella header. They were kept in
//! sync by hand and nothing checked them.
//!
//! A header that disagrees with the dylib is not a build failure on the
//! Swift side. The symbol still links; the call is simply made with the
//! wrong argument shape, at runtime, in a shipped app. So comparing the two
//! copies to each other is not enough -- they can agree and both be wrong.
//! This test parses `src/lib.rs` for every `#[unsafe(no_mangle)] extern "C"`
//! function and requires BOTH headers to declare exactly that set with
//! exactly those signatures.
//!
//! The parsers here are deliberately strict: anything they cannot account
//! for is a panic, not a skip. A guard that quietly parses nothing is worse
//! than no guard.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// A signature reduced to the only thing the ABI cares about: the C types,
/// in order. Parameter names and formatting are dropped, so a purely
/// cosmetic reflow of a declaration does not fail this test.
type Signature = (String, Vec<String>);

fn ffi_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    // <repo>/crates/trace-commons-contributor-ffi
    ffi_crate_dir()
        .parent()
        .and_then(|p| p.parent())
        .expect("crate is two directories below the repo root")
        .to_path_buf()
}

fn header_paths() -> Vec<PathBuf> {
    vec![
        ffi_crate_dir().join("include/trace_commons.h"),
        repo_root().join("macos/Sources/CTraceCommons/include/trace_commons.h"),
    ]
}

// ---------------------------------------------------------------------------
// Rust side
// ---------------------------------------------------------------------------

/// Map one Rust FFI type onto its C spelling. Panics on anything not already
/// in the ABI: a new pointer shape should be added here deliberately, not
/// waved through by a permissive fallback.
fn rust_type_to_c(ty: &str) -> String {
    let ty = ty.trim();
    if let Some(inner) = ty.strip_prefix("*const ") {
        let inner = inner.trim();
        if inner.starts_with('*') {
            return format!("{}*", rust_type_to_c(inner));
        }
        return format!("const {}*", rust_base_type_to_c(inner));
    }
    if let Some(inner) = ty.strip_prefix("*mut ") {
        let inner = inner.trim();
        if inner.starts_with('*') {
            return format!("{}*", rust_type_to_c(inner));
        }
        return format!("{}*", rust_base_type_to_c(inner));
    }
    if ty.starts_with("Option<") {
        return rust_fn_pointer_to_c(ty);
    }
    rust_base_type_to_c(ty)
}

fn rust_base_type_to_c(ty: &str) -> String {
    match ty.trim() {
        "c_char" => "char",
        "c_void" => "void",
        "tc_handle" => "tc_handle",
        "tc_preview" => "tc_preview",
        "u64" => "uint64_t",
        "u32" => "uint32_t",
        "i64" => "int64_t",
        "i32" => "int32_t",
        "()" | "" => "void",
        other => panic!(
            "unmapped Rust type `{other}` on the C ABI surface -- add it to \
             rust_base_type_to_c rather than loosening this test"
        ),
    }
    .to_string()
}

/// `Option<extern "C" fn(event_json: *const c_char, ctx: *mut c_void)>`
/// becomes `void(*)(const char*,void*)`.
fn rust_fn_pointer_to_c(ty: &str) -> String {
    let inner = ty
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or_else(|| panic!("unsupported Option type on the ABI surface: `{ty}`"));
    let inner = inner.trim();
    let inner = inner
        .strip_prefix("extern \"C\" fn")
        .unwrap_or_else(|| panic!("Option on the ABI surface is not a fn pointer: `{inner}`"));
    let (params, rest) = split_parens(inner);
    let ret = match rest.trim().strip_prefix("->") {
        Some(r) => rust_type_to_c(r),
        None => "void".to_string(),
    };
    let params = split_top_level_commas(&params)
        .into_iter()
        .map(|p| rust_type_to_c(&strip_rust_param_name(&p)))
        .collect::<Vec<_>>();
    format!("{ret}(*)({})", params.join(","))
}

fn strip_rust_param_name(param: &str) -> String {
    match param.split_once(':') {
        Some((_name, ty)) => ty.trim().to_string(),
        None => param.trim().to_string(),
    }
}

/// Take the text between the first balanced `(` and its `)`, returning
/// (inside, remainder).
fn split_parens(s: &str) -> (String, String) {
    let open = s.find('(').unwrap_or_else(|| panic!("no `(` in `{s}`"));
    let mut depth = 0usize;
    for (i, ch) in s.char_indices().skip(open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (s[open + 1..i].to_string(), s[i + 1..].to_string());
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced parentheses in `{s}`")
}

fn split_top_level_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '(' | '<' => {
                depth += 1;
                current.push(ch);
            }
            ')' | '>' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                out.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out.retain(|p| !p.is_empty());
    out
}

fn rust_surface() -> BTreeMap<String, Signature> {
    let src = std::fs::read_to_string(ffi_crate_dir().join("src/lib.rs"))
        .expect("failed to read the FFI crate's src/lib.rs");

    let mut out: BTreeMap<String, Signature> = BTreeMap::new();
    let bytes: Vec<&str> = src.lines().collect();
    for (i, line) in bytes.iter().enumerate() {
        if line.trim() != "#[unsafe(no_mangle)]" {
            continue;
        }
        // The declaration may wrap over several lines; take everything up to
        // the opening brace of the body.
        let mut decl = String::new();
        for line in bytes.iter().skip(i + 1) {
            decl.push(' ');
            decl.push_str(line.trim());
            if line.contains('{') {
                break;
            }
        }
        let decl = decl.trim().to_string();
        if !decl.contains("extern \"C\" fn") {
            panic!("`#[unsafe(no_mangle)]` on a non-`extern \"C\"` item: `{decl}`");
        }
        let after_fn = decl
            .split_once("extern \"C\" fn")
            .expect("checked above")
            .1
            .trim_start();
        let name_end = after_fn
            .find('(')
            .unwrap_or_else(|| panic!("no parameter list in `{decl}`"));
        let name = after_fn[..name_end].trim().to_string();

        let (params, rest) = split_parens(after_fn);
        let ret = rest
            .split_once('{')
            .map(|(before, _)| before)
            .unwrap_or(&rest);
        let ret = match ret.trim().strip_prefix("->") {
            Some(r) => rust_type_to_c(r),
            None => "void".to_string(),
        };
        let params: Vec<String> = split_top_level_commas(&params)
            .into_iter()
            .map(|p| rust_type_to_c(&strip_rust_param_name(&p)))
            .collect();

        if out.insert(name.clone(), (ret, params)).is_some() {
            panic!("`{name}` is exported twice from the FFI crate");
        }
    }

    assert!(
        !out.is_empty(),
        "parsed zero exported symbols from src/lib.rs -- the parser has gone \
         blind, which would make this whole test vacuous"
    );
    out
}

// ---------------------------------------------------------------------------
// Header side
// ---------------------------------------------------------------------------

fn strip_c_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '/' {
            match chars.peek() {
                Some('*') => {
                    chars.next();
                    let mut prev = '\0';
                    for c in chars.by_ref() {
                        if prev == '*' && c == '/' {
                            break;
                        }
                        prev = c;
                    }
                    out.push(' ');
                    continue;
                }
                Some('/') => {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                    out.push('\n');
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
    }
    out
}

/// Reduce a C parameter or return type to a canonical spelling: no
/// parameter name, no incidental whitespace, `*` bound to the type.
fn canonical_c_type(decl: &str) -> String {
    let decl = decl.trim();
    if decl.is_empty() {
        panic!("empty C type");
    }
    // Function pointer: `void (*cb)(const char* a, void* b)`.
    if let Some(star_paren) = decl.find("(*") {
        let ret = canonical_c_type(&decl[..star_paren]);
        let after = &decl[star_paren..];
        let (_name, rest) = split_parens(after);
        let (params, _) = split_parens(&rest);
        let params: Vec<String> = split_top_level_commas(&params)
            .into_iter()
            .map(|p| canonical_c_type(&p))
            .collect();
        return format!("{ret}(*)({})", params.join(","));
    }

    let spaced = decl.replace('*', " * ");
    let mut is_const = false;
    let mut base: Option<String> = None;
    let mut stars = 0usize;
    let mut dropped: Vec<String> = Vec::new();
    for token in spaced.split_whitespace() {
        match token {
            "const" => is_const = true,
            "*" => stars += 1,
            "struct" => {}
            "char" | "void" | "int" | "int8_t" | "int16_t" | "int32_t" | "int64_t" | "uint8_t"
            | "uint16_t" | "uint32_t" | "uint64_t" | "size_t" | "tc_handle" | "tc_preview" => {
                if let Some(existing) = &base {
                    panic!("two base types (`{existing}`, `{token}`) in C type `{decl}`");
                }
                base = Some(token.to_string());
            }
            other => dropped.push(other.to_string()),
        }
    }
    let base = base.unwrap_or_else(|| panic!("no recognised base type in C type `{decl}`"));
    assert!(
        dropped.len() <= 1,
        "unparsed tokens {dropped:?} in C type `{decl}` -- extend \
         canonical_c_type rather than loosening it"
    );
    let mut out = String::new();
    if is_const {
        out.push_str("const ");
    }
    out.push_str(&base);
    for _ in 0..stars {
        out.push('*');
    }
    out
}

fn header_surface(path: &PathBuf) -> BTreeMap<String, Signature> {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let src = strip_c_comments(&src);

    let mut out: BTreeMap<String, Signature> = BTreeMap::new();
    for statement in src.split(';') {
        let statement = statement
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join(" ");
        let statement = statement.trim();
        if statement.is_empty() || !statement.contains('(') {
            continue;
        }
        if statement.starts_with("typedef") || statement.contains('}') {
            continue;
        }
        let (params, _rest) = split_parens(statement);
        let head = &statement[..statement.find('(').expect("checked above")];
        // `tc_handle*  tc_daemon_start` -- the last identifier is the name.
        let name_start = head
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|i| i + 1)
            .unwrap_or(0);
        let name = head[name_start..].trim().to_string();
        if name.is_empty() {
            panic!("could not find a function name in `{statement}`");
        }
        let ret = canonical_c_type(&head[..name_start]);
        let params: Vec<String> = split_top_level_commas(&params)
            .into_iter()
            .map(|p| canonical_c_type(&p))
            .filter(|p| p != "void")
            .collect();
        if out.insert(name.clone(), (ret, params)).is_some() {
            panic!("`{name}` is declared twice in {}", path.display());
        }
    }

    assert!(
        !out.is_empty(),
        "parsed zero declarations from {} -- the parser has gone blind",
        path.display()
    );
    out
}

fn render(name: &str, sig: &Signature) -> String {
    format!("{} {name}({})", sig.0, sig.1.join(", "))
}

/// Line-per-symbol difference report. `assert_eq!` on two 18-line strings
/// prints them escaped onto one line each and leaves the reader to spot the
/// changed character; this points at the symbol.
fn differences(
    actual: &BTreeMap<String, Signature>,
    actual_label: &str,
    expected: &BTreeMap<String, Signature>,
    expected_label: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for (name, sig) in expected {
        match actual.get(name) {
            None => out.push(format!(
                "  {name}: declared in {expected_label} as `{}`, MISSING from {actual_label}",
                render(name, sig)
            )),
            Some(other) if other != sig => out.push(format!(
                "  {name}:\n    {expected_label}: {}\n    {actual_label}: {}",
                render(name, sig),
                render(name, other)
            )),
            Some(_) => {}
        }
    }
    for (name, sig) in actual {
        if !expected.contains_key(name) {
            out.push(format!(
                "  {name}: declared in {actual_label} as `{}`, MISSING from {expected_label}",
                render(name, sig)
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

#[test]
fn every_header_copy_matches_the_rust_abi_surface() {
    let rust = rust_surface();
    for path in header_paths() {
        let header = header_surface(&path);
        let diff = differences(&header, "header", &rust, "src/lib.rs");
        assert!(
            diff.is_empty(),
            "\n{} disagrees with the `extern \"C\"` surface in \
             crates/trace-commons-contributor-ffi/src/lib.rs:\n\n{}\n\n\
             The Rust is the ground truth: it is what ships in the dylib, and \
             a header that disagrees is a wrong call at runtime, not a link \
             error. Update the header -- or if the Rust changed deliberately, \
             update BOTH copies this test checks.\n",
            path.display(),
            diff.join("\n")
        );
    }
}

#[test]
fn the_two_header_copies_declare_the_same_surface() {
    let paths = header_paths();
    let first = header_surface(&paths[0]);
    for path in &paths[1..] {
        let diff = differences(&header_surface(path), "macos copy", &first, "ffi copy");
        assert!(
            diff.is_empty(),
            "\n{} and {} declare different C ABIs:\n\n{}\n\nThe macOS Swift \
             package names its own copy as the `CTraceCommons` umbrella \
             header, so a divergence here is what the app compiles against.\n",
            path.display(),
            paths[0].display(),
            diff.join("\n")
        );
    }
}

/// The two copies are held byte-identical, not merely ABI-equivalent.
///
/// The semantic checks above are what protect the ABI; this one protects the
/// prose. The copies had already drifted in their documentation -- one had
/// lost a paragraph about where a preview's redacted envelope is stored, and
/// carried an older rewording of `tc_daemon_start_with_settings` -- which is
/// how a reader ends up trusting the wrong description of a boundary whose
/// whole job is stating ownership and failure rules precisely.
///
/// Byte equality also makes the fix unambiguous: copy the FFI crate's file
/// over the macOS one. It is the closest this layout gets to a single source
/// of truth without a symlink, which this repo uses nowhere and which a
/// Windows checkout would silently materialise as a text file.
#[test]
fn the_macos_copy_is_a_byte_for_byte_copy() {
    let paths = header_paths();
    let canonical = std::fs::read_to_string(&paths[0]).expect("failed to read the FFI header");
    for path in &paths[1..] {
        let other = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        assert!(
            other == canonical,
            "\n{} is not byte-identical to {}.\n\nThe FFI crate's copy is \
             canonical -- it sits beside the Rust it describes. To resync:\n\n    \
             cp crates/trace-commons-contributor-ffi/include/trace_commons.h \\\n       \
             macos/Sources/CTraceCommons/include/trace_commons.h\n",
            path.display(),
            paths[0].display()
        );
    }
}
