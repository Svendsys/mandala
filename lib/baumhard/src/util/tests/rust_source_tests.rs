// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::util::rust_source`], the reader every
//! source-level pin in the workspace goes through.
//!
//! These are the tests that matter most in the module, because a
//! stripper that quietly does nothing turns every pin built on it
//! into a test that cannot fail — the exact failure
//! `crate::util::manifests` warns about for its own parser. Each
//! case below therefore asserts both halves: the comment is gone
//! *and* the code around it survived.

use crate::util::rust_source::{above_test_modules, braced_block_after, production_code, strip_comments};

/// Every comment shape in the workspace disappears; nothing that is
/// not a comment moves.
///
/// The `(input, must_not_contain, must_contain)` triple is the whole
/// contract: a stripper that returned `String::new()` would satisfy
/// the first column and fail the second, and one that returned its
/// input unchanged would do the reverse.
pub fn do_strip_comments_removes_only_comments() {
    let cases: &[(&str, &[&str], &[&str])] = &[
        // Line comments, including both doc forms — a stale needle
        // is likeliest to survive in a doc comment.
        ("let a = 1; // log::error!\n", &["log::error!"], &["let a = 1;"]),
        ("/// log::error!\nlet a = 1;", &["log::error!"], &["let a = 1;"]),
        ("//! log::error!\nlet a = 1;", &["log::error!"], &["let a = 1;"]),
        // Block comments, including the nested form Rust allows. A
        // stripper that stopped at the first `*/` would leave
        // `keep_me */` behind.
        ("a /* log::error! */ b", &["log::error!"], &["a", "b"]),
        (
            "a /* x /* log::error! */ keep_me */ b",
            &["log::error!", "keep_me"],
            &["a", "b"],
        ),
        // A `//` inside a string is not a comment. This is the case
        // that makes the pins possible at all: the needles are
        // string literals.
        (
            r#"log::error!("startup: {}", message); // gone"#,
            &["gone"],
            [r#"log::error!("startup: {}", message);"#].as_slice(),
        ),
        (r#"let u = "http://x"; // gone"#, &["gone"], &["http://x"]),
        // Raw strings: a backslash escapes nothing, and only the
        // matching hash run closes them.
        (r##"let s = r#"a // b "# ; // gone"##, &["gone"], &["a // b"]),
        (r#"let s = r"c:\path"; // gone"#, &["gone"], &[r"c:\path"]),
        // `'` opens a lifetime far more often than a literal, and
        // mistaking one for the other swallows the rest of the file.
        (
            "fn f<'a>(x: &'a str) -> &'a str { x } // gone",
            &["gone"],
            &["fn f<'a>(x: &'a str)"],
        ),
        ("let c = '/'; let d = 1; // gone", &["gone"], &["let d = 1;"]),
        ("let c = '\\''; let d = 1; // gone", &["gone"], &["let d = 1;"]),
        // A comment is replaced, not deleted, so the tokens either
        // side cannot fuse into one.
        ("a/*x*/b", &["x"], &["a b"]),
    ];
    // The replacement is exactly one space, in both comment shapes.
    // For a block comment that is load-bearing — `a/*x*/b` must not
    // become `ab` — and for a line comment it is the documented
    // contract, which the newline would otherwise make unobservable.
    assert_eq!(strip_comments("a // b\nc"), "a  \nc");
    assert_eq!(strip_comments("a/*x*/b"), "a b");

    for (input, absent, present) in cases {
        let stripped = strip_comments(input);
        for needle in *absent {
            assert!(
                !stripped.contains(needle),
                "{needle:?} survived stripping of {input:?}: {stripped:?}"
            );
        }
        for needle in *present {
            assert!(
                stripped.contains(needle),
                "{needle:?} was destroyed by stripping of {input:?}: {stripped:?}"
            );
        }
    }
}

/// Line count is preserved, so a byte offset into the output still
/// lands on the line it came from and a failure message can quote a
/// line number that means something.
pub fn do_strip_comments_preserves_line_count() {
    for input in [
        "a\n// one\nb\n",
        "a\n/* one\n two\n three */\nb\n",
        "/// doc\n/// doc\nfn f() {}\n",
        "",
        "no trailing newline // gone",
    ] {
        assert_eq!(
            strip_comments(input).lines().count(),
            input.lines().count(),
            "line count moved for {input:?}"
        );
    }
}

/// An unterminated comment or literal must terminate the scan rather
/// than run off the end of the string — a truncated file being read
/// mid-write is the realistic way this is reached.
pub fn do_strip_comments_survives_unterminated_input() {
    for input in [
        "a /* never closed",
        "a \"never closed",
        "a r#\"never closed",
        "a '",
    ] {
        let _ = strip_comments(input);
    }
    assert!(strip_comments("keep /* gone").contains("keep"));
    assert!(!strip_comments("keep /* gone").contains("gone"));
}

/// An *inline* test module ends the production half; nothing else
/// does.
///
/// The negative cases are the ones that matter, and each cost a real
/// pin before it was handled. Truncating early returns a shorter
/// string — in both cases here, an empty one — and an empty string
/// satisfies every negative assertion and no positive one: a scan
/// that silently cannot fail.
///
/// - `src/application/renderer/mod.rs` opens with a `#[cfg(test)]
///   use …` five hundred lines above the code its pin is about.
/// - `src/application/document/mod.rs` declares fourteen
///   `#[cfg(test)] mod tests_*;` **before its first real item**.
///   None of that text is in the file, so there is nothing to cut.
pub fn do_above_test_modules_cuts_at_the_module_only() {
    let attribute = concat!("#[cfg(", "test)]");

    let early_use = format!("{attribute}\nuse helper::thing;\nfn ships() {{ keep(); }}\n");
    assert!(
        above_test_modules(&early_use).contains("fn ships()"),
        "a test-only import must not truncate the file"
    );

    let helper = format!("{attribute}\nfn only_in_tests() {{}}\nfn ships() {{ keep(); }}\n");
    assert!(
        above_test_modules(&helper).contains("fn ships()"),
        "a test-only helper fn must not truncate the file"
    );

    let external =
        format!("{attribute}\nmod tests_nodes;\n{attribute}\nmod tests_edges;\nfn ships() {{ keep(); }}\n");
    assert!(
        above_test_modules(&external).contains("fn ships()"),
        "an external test-module declaration holds no text in this file and must not truncate it"
    );

    for module in [
        "mod tests {",
        "pub mod tests {",
        "pub(crate) mod tests {",
        "pub(super) mod tests {",
    ] {
        let src =
            format!("fn ships() {{ keep(); }}\n{attribute}\n{module}\n  fn hidden() {{ needle(); }}\n}}\n");
        let production = above_test_modules(&src);
        assert!(
            production.contains("fn ships()"),
            "{module}: production code was cut away"
        );
        assert!(
            !production.contains("needle()"),
            "{module}: the test module survived"
        );
    }

    // Declarations first, then an inline module: the inline one is
    // still the cut, and the declarations above it do not move it.
    let mixed = format!(
        "{attribute}\nmod tests_nodes;\nfn ships() {{ keep(); }}\n{attribute}\nmod tests {{\n  fn hidden() {{ needle(); }}\n}}\n"
    );
    let production = above_test_modules(&mixed);
    assert!(
        production.contains("fn ships()"),
        "mixed: production code was cut away"
    );
    assert!(
        !production.contains("needle()"),
        "mixed: the inline module survived"
    );

    // No test module at all: the whole file is production.
    assert_eq!(above_test_modules("fn ships() {}"), "fn ships() {}");
}

/// The body of one item, brace-matched — and a brace inside a string
/// does not close it early. Without that, pinning a statement to a
/// function whose body formats `"{}"` would read the wrong span.
pub fn do_braced_block_after_matches_one_item() {
    let src = "fn a() { one(); }\nfn adopt(s: S) -> D { log::error!(\"x: {}\", m); d }\nfn b() { two(); }";
    let body = braced_block_after(src, "fn adopt(").expect("adopt is present");
    assert!(
        body.contains("log::error!"),
        "body missed its statement: {body:?}"
    );
    assert!(!body.contains("one()"), "body reached backwards: {body:?}");
    assert!(!body.contains("two()"), "body ran past its close: {body:?}");

    // Nested blocks close in the right order.
    let nested = "fn f() { if x { inner(); } tail(); }\nfn g() { other(); }";
    let body = braced_block_after(nested, "fn f(").expect("f is present");
    assert!(body.contains("inner();") && body.contains("tail();"));
    assert!(!body.contains("other();"), "nesting mis-counted: {body:?}");

    // A closing brace inside a literal is not a closing brace.
    let literal = r#"fn f() { let s = "}"; let c = '}'; tail(); }"#;
    let body = braced_block_after(literal, "fn f(").expect("f is present");
    assert!(
        body.contains("tail();"),
        "a literal brace closed the block: {body:?}"
    );

    assert!(braced_block_after(src, "fn absent(").is_none());
}

/// The reader, end to end, against a file that is in the tree and
/// stays there: this module's own source.
///
/// Two properties, both of which a naive `read_to_string` fails: the
/// module's prose — which quotes the very needles the pins search
/// for — does not reach the caller, and the code does.
pub fn do_production_code_returns_code_without_prose() {
    let code = production_code("lib/baumhard/src/util/rust_source.rs");
    assert!(
        code.contains("pub fn strip_comments(src: &str) -> String"),
        "production_code dropped the code it was asked for"
    );
    assert!(
        !code.contains("A **string literal** in production code"),
        "production_code leaked its own module documentation"
    );
    // The module doc quotes this needle in prose. If prose reached
    // the caller, every pin built on this reader would be satisfiable
    // by a comment — the exact hazard the module exists to close.
    assert!(
        !code.contains(r#"log::error!("startup: {}", message)"#),
        "a needle quoted in a doc comment reached the caller"
    );
}

#[test]
fn test_strip_comments_removes_only_comments() {
    do_strip_comments_removes_only_comments();
}

#[test]
fn test_strip_comments_preserves_line_count() {
    do_strip_comments_preserves_line_count();
}

#[test]
fn test_strip_comments_survives_unterminated_input() {
    do_strip_comments_survives_unterminated_input();
}

#[test]
fn test_above_test_modules_cuts_at_the_module_only() {
    do_above_test_modules_cuts_at_the_module_only();
}

#[test]
fn test_braced_block_after_matches_one_item() {
    do_braced_block_after_matches_one_item();
}

#[test]
fn test_production_code_returns_code_without_prose() {
    do_production_code_returns_code_without_prose();
}
