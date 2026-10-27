// SPDX-License-Identifier: MPL-2.0

//! The loader is handed untrusted bytes, so it must answer any input
//! with a document or an error, never a panic, and a document it
//! accepts must save to text that loads and saves to the same text.
//! This is a deterministic stand-in for a fuzzer (V1-D29): the same
//! property, driven over mutations of the fixtures.

use crate::document::format::{load, load_bytes_under, load_from_bytes, LoadErrorKind};
use crate::document::tests::format_support::{same_document, written, FIXTURES};

/// A small deterministic generator (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }
}

/// Fragments that change the meaning of a JSON text rather than
/// breaking it.
const TOKENS: &[&str] = &[
    "null",
    "true",
    "[",
    "]",
    "{",
    "}",
    ",",
    ":",
    "\"",
    "0",
    "-1",
    "1e308",
    "1e-45",
    "4294967295",
    "\"var(x)\"",
    "\"#12345678\"",
    "[0,0]",
    "{}",
    "[]",
    "\"\"",
    "18446744073709551615",
    "-0.0",
    "0.5",
    "\u{00e9}",
    "\\u0000",
];

/// `bytes`, mutated once.
fn mutate(bytes: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = bytes.to_vec();
    if out.is_empty() {
        return out;
    }
    let at = rng.below(out.len());
    match rng.below(6) {
        0 => out[at] = (rng.next() & 0xFF) as u8,
        1 => {
            let end = (at + 1 + rng.below(40)).min(out.len());
            out.drain(at..end);
        }
        2 => {
            let end = (at + 1 + rng.below(80)).min(out.len());
            let copy = out[at..end].to_vec();
            let to = rng.below(out.len());
            out.splice(to..to, copy);
        }
        3 => out.truncate(at),
        _ => {
            let token = TOKENS[rng.below(TOKENS.len())];
            // Replace a short run with a token, or insert it.
            let end = if rng.below(2) == 0 {
                at
            } else {
                (at + 1 + rng.below(8)).min(out.len())
            };
            out.splice(at..end, token.bytes());
        }
    }
    out
}

#[test]
fn test_no_input_makes_the_loader_panic_and_what_loads_round_trips() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let (mut loaded, mut refused) = (0, 0);
    for (name, text) in FIXTURES {
        for _ in 0..1500 {
            let mut bytes = mutate(text.as_bytes(), &mut rng);
            // Sometimes mutate twice: a refusal after a refusal's cause.
            if rng.below(3) == 0 {
                bytes = mutate(&bytes, &mut rng);
            }
            match load_from_bytes(&bytes) {
                Ok(doc) => {
                    loaded += 1;
                    let saved = written(&doc);
                    let again = load(&saved).unwrap_or_else(|error| {
                        panic!("{name}: what loaded saved to text that does not: {error}")
                    });
                    same_document(&doc, &again)
                        .unwrap_or_else(|why| panic!("{name}: load, save, load changed the map: {why}"));
                    assert_eq!(written(&again), saved, "{name}: save is not a fixed point");
                }
                Err(error) => {
                    refused += 1;
                    // Every refusal but the whole-text ones names a
                    // place. The map itself has the empty path: its own
                    // missing key, a wrong top-level type, and a key
                    // that is the empty string (whose path is the
                    // object's, with nothing after it).
                    if !matches!(
                        error.kind,
                        LoadErrorKind::Syntax { .. } | LoadErrorKind::TooLarge { .. }
                    ) {
                        let at_the_map = matches!(
                            &error.kind,
                            LoadErrorKind::MissingKey { .. }
                                | LoadErrorKind::Invalid { .. }
                                | LoadErrorKind::DuplicateKey { .. }
                        ) || matches!(&error.kind, LoadErrorKind::UnknownKey { key } if key.is_empty());
                        assert!(
                            !error.path.is_empty() || at_the_map,
                            "{name}: a refusal without a path: {error:?}"
                        );
                    }
                }
            }
        }
    }
    assert!(loaded > 100, "precondition: {loaded} mutations still loaded");
    assert!(refused > 5000, "precondition: {refused} mutations were refused");
}

#[test]
fn test_bytes_that_are_not_text_are_refused_not_trusted() {
    for bytes in [
        &[0xFF, 0xFE, 0x00][..],
        &[b'{', 0xC0, 0x80, b'}'][..],
        &[0x80][..],
    ] {
        let error = load_from_bytes(bytes).unwrap_err();
        assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
        assert_eq!(error.path, "");
    }
    assert!(load_from_bytes(&[]).is_err());
    // The size ceiling is applied to bytes before they are read as text.
    let error = load_bytes_under(&[0xFF; 64], Some(8)).unwrap_err();
    assert!(
        matches!(error.kind, LoadErrorKind::TooLarge { bytes: 64, max: 8 }),
        "{error:?}"
    );
}

#[test]
fn test_hostile_nesting_is_an_error_not_a_stack_overflow() {
    // A million opening brackets: refused by shape, at the first.
    let deep = "[".repeat(1_000_000);
    assert!(load(&deep).is_err());
    // 100,000 levels of arrays inside one node, closed: the index skips
    // them without recursing, and the node is refused at the first value
    // its schema has no room for, never by the stack.
    let nested = |inner: &str| {
        format!(
            r#"{{"format":1,"name":"t","nodes":[{{"id":1,"position":[0,0],"sections":{}{inner}{}}}]}}"#,
            "[".repeat(100_000),
            "]".repeat(100_000)
        )
    };
    let error = load(&nested("")).unwrap_err();
    assert!(error.path.starts_with("nodes[0].sections"), "{error:?}");
    // The same, left open: a truncated text, found by the index.
    let open = format!(
        r#"{{"format":1,"name":"t","nodes":[{{"id":1,"position":[0,0],"sections":{}"#,
        "[".repeat(100_000)
    );
    let error = load(&open).unwrap_err();
    assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
    // And inside a top-level value the format does not define, which is
    // skipped, then refused by name once `format` has been read.
    let unknown = format!(
        r#"{{"format":1,"name":"t","bogus":{}{}}}"#,
        "[".repeat(100_000),
        "]".repeat(100_000)
    );
    let error = load(&unknown).unwrap_err();
    assert_eq!(
        (error.path.as_str(), &error.kind),
        (
            "bogus",
            &LoadErrorKind::UnknownKey {
                key: "bogus".to_string()
            }
        )
    );
    // And one the deserializer does follow to the end, with a long
    // run of spans: a size the loader pays for linearly.
    let spans = r#"{"text":"x"},"#.repeat(100_000);
    let json = format!(
        r##"{{"format":1,"name":"t","theme":{{"active":"d","variants":{{"d":{{}}}}}},"nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{spans}{{"text":"y"}}]}}]}}]}}"##
    );
    assert!(load(&json).is_ok());
}
