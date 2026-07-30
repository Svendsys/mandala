// SPDX-License-Identifier: MPL-2.0

// The module-level concept header for this file lives as a `///`
// doc on `pub mod name_rules;` in `src/font/mod.rs` rather than as
// a `//!` header here: `build.rs` pulls this file in with
// `include!`, and an inner doc comment cannot appear in a macro
// expansion. Rustdoc renders the two forms identically.

/// File extensions the font scan accepts, lowercase. Compared
/// case-insensitively by [`is_font_extension`] so `Foo.TTF` is not
/// silently skipped on a case-preserving filesystem.
///
/// Order is meaningful: it is the preference order used to break a
/// collision between two files that describe the same font, so
/// `TreeRoot-x.ttf` wins over `Treeroot-y.otf`.
pub const FONT_EXTENSIONS: [&str; 2] = ["ttf", "otf"];

/// Shortest token [`fallback_sanitize`] keeps when a font's `name`
/// table yields nothing usable and the file stem has to carry the
/// variant name. Tokens shorter than this are noise (`v2`, `a`, the
/// random suffix a font host appends) — unless dropping them would
/// leave no name at all.
pub const MIN_TOKEN_LEN: usize = 4;

/// Length at which [`fallback_sanitize`] treats a stem's first
/// token as the whole name and ignores what follows.
///
/// Font hosts ship `ToyTrain-WYdO.ttf`: a real name followed by a
/// cache-busting suffix. A first token this long is the name, and
/// gluing the suffix onto it would only produce `ToyTrainWYdO`.
/// Below the threshold the token is too generic to stand alone and
/// the remaining tokens are joined onto it.
pub const SELF_SUFFICIENT_TOKEN_LEN: usize = 5;

/// Longest variant name [`fallback_sanitize`] will build out of a
/// file stem. Font hosts hand out stems like
/// `some-extremely-long-display-face-name-a1b2c3`; without a
/// ceiling the generated enum grows unreadable variants.
pub const MAX_FALLBACK_LEN: usize = 20;

/// Name of the "no preference" sentinel variant. Reserved: a font
/// whose derived name collides with it is renamed by
/// [`select_font_variants`] rather than allowed to produce a
/// duplicate variant.
pub const ANY_VARIANT: &str = "Any";

/// Text encoding of a raw OpenType `name`-table record.
///
/// The `name` table stores one string per (platform, encoding,
/// language) triple, and the encoding is *not* implied by the
/// bytes: a Windows-platform record is UTF-16BE, a
/// Macintosh-platform record is a legacy single-byte codepage.
/// Decoding a UTF-16BE record as UTF-8 happens to work for pure
/// ASCII names — every other byte is `0x00`, which is valid UTF-8
/// and filtered out downstream — and fails the moment a name
/// contains a single accented character. [`decode_name_record`]
/// takes the encoding explicitly so that accident cannot be
/// mistaken for correctness.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NameEncoding {
    /// UTF-16 big-endian: Unicode platform, or Windows platform
    /// with the Symbol / Unicode-BMP encoding IDs. This is what
    /// `ttf_parser::name::Name::is_unicode` reports.
    Utf16Be,
    /// Anything else — in practice Macintosh Roman. Bytes below
    /// `0x80` are ASCII; the high half is a legacy codepage whose
    /// characters [`ascii_font_name`] discards anyway, so they are
    /// decoded to the replacement character rather than through a
    /// codepage table that would only feed the filter.
    Legacy,
}

/// Decode one raw `name`-table record into a Rust `String`.
///
/// Never fails and never panics: malformed UTF-16 (unpaired
/// surrogates, an odd byte count) and legacy high bytes both decode
/// to `U+FFFD`. That matters because this runs inside a build
/// script — a panic here is a build failure with no recovery path,
/// and the whole point of the fallback chain in [`variant_name`] is
/// that an undecodable name degrades to the file stem instead of
/// stopping the build.
///
/// Costs: one `String` allocation, one pass over `bytes`.
pub fn decode_name_record(bytes: &[u8], encoding: NameEncoding) -> String {
    match encoding {
        NameEncoding::Utf16Be => {
            let units = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
            char::decode_utf16(units)
                .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }
        NameEncoding::Legacy => bytes
            .iter()
            .map(|&byte| {
                if byte.is_ascii() {
                    byte as char
                } else {
                    char::REPLACEMENT_CHARACTER
                }
            })
            .collect(),
    }
}

/// Reduce a decoded font name to the ASCII alphanumerics and single
/// spaces a Rust identifier can be built from.
///
/// Rust identifiers may contain non-ASCII XID characters, but an
/// `AppFont::Café` variant is a trap for every downstream consumer
/// (source encoding, `grep`, generated bindings), so the enum stays
/// ASCII. Whitespace runs collapse to one space and the result is
/// trimmed, leaving [`camel_case`] a clean word sequence to
/// capitalize.
///
/// Costs: one `String` allocation, one pass over `decoded`.
pub fn ascii_font_name(decoded: &str) -> String {
    let mut out = String::with_capacity(decoded.len());
    let mut pending_space = false;
    for ch in decoded.chars() {
        if ch.is_whitespace() {
            pending_space = !out.is_empty();
        } else if ch.is_ascii_alphanumeric() {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Return the extension's lowercase form when it names a font
/// container we compile in, `None` otherwise. Case-insensitive, so
/// `Foo.TTF` and `Foo.ttf` are the same kind of file.
pub fn is_font_extension(extension: &str) -> Option<String> {
    let lowered = extension.to_ascii_lowercase();
    FONT_EXTENSIONS.contains(&lowered.as_str()).then_some(lowered)
}

/// Preference rank of a font container, lower wins. Used to break a
/// collision between two files describing the same font: the TTF is
/// preferred over the OTF, matching the historical behavior of the
/// scan. An unknown extension ranks last.
pub fn extension_rank(extension: &str) -> usize {
    FONT_EXTENSIONS
        .iter()
        .position(|candidate| *candidate == extension)
        .unwrap_or(FONT_EXTENSIONS.len())
}

/// Family grouping key for a font file: the lowercased file-stem
/// prefix up to the first `-`.
///
/// Font hosts ship the same face as both a TTF and an OTF with the
/// same stem prefix and different random suffixes
/// (`AppleTea-jELql.otf` / `AppleTea-z8R1a.ttf`). Both describe one
/// font, so only one becomes an `AppFont` variant; this key is what
/// groups them.
pub fn family_key(file_stem: &str) -> String {
    file_stem
        .split_once('-')
        .map_or(file_stem, |(prefix, _)| prefix)
        .to_ascii_lowercase()
}

/// Uppercase the first character of `word`, leaving the rest as-is.
///
/// Deliberately *not* a lowercasing pass over the tail: font names
/// carry meaningful internal capitalization (`GalatiaSIL`,
/// `RCRocket`, `NIGHTCROW`) that a naive title-case would destroy.
pub fn capitalize_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Move a run of leading ASCII digits to the end of `name`, so
/// `212Keyboard` becomes `Keyboard212`.
///
/// A Rust identifier cannot start with a digit, and several fonts
/// in the tree do (`212 Keyboard`, `2Peas Hearts Delight`).
/// Rotating rather than dropping keeps the digits — they are part
/// of the font's identity — while producing a legal variant name.
pub fn rotate_leading_digits(name: &str) -> String {
    let split = name.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(name.len());
    if split == 0 {
        return name.to_string();
    }
    format!("{}{}", &name[split..], &name[..split])
}

/// Turn an [`ascii_font_name`] result into a Rust enum-variant
/// identifier, or `None` when the name carries nothing an
/// identifier can be built from (empty, whitespace only, digits
/// only).
///
/// Words are joined without a separator and each word's first
/// character is uppercased; leading digits are rotated to the end
/// by [`rotate_leading_digits`]. The input is expected to be the
/// output of [`ascii_font_name`] — ASCII alphanumerics separated by
/// single spaces — so that the "which characters survive" decision
/// lives in exactly one place.
///
/// Costs: two `String` allocations, one pass per word.
pub fn camel_case(ascii_name: &str) -> Option<String> {
    let joined: String = ascii_name.split_whitespace().map(capitalize_first).collect();
    let rotated = rotate_leading_digits(&joined);
    if rotated.is_empty() || rotated.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(rotated)
}

/// Derive a variant name from a font file's stem, for files whose
/// `name` table is missing, undecodable, or carries nothing usable.
///
/// The stem is split on whitespace, `-`, and `_`, and each token is
/// reduced to ASCII alphanumerics. Tokens shorter than
/// [`MIN_TOKEN_LEN`] are dropped as noise — version markers, the
/// random suffix font hosts append — unless dropping them would
/// leave nothing. Then:
///
/// - a first token of at least [`SELF_SUFFICIENT_TOKEN_LEN`]
///   characters *is* the name (`ToyTrain-WYdO` → `ToyTrain`, not
///   `ToyTrainWYdO`);
/// - otherwise the tokens are capitalized and joined while the
///   result fits [`MAX_FALLBACK_LEN`].
///
/// Either way the result is capped at [`MAX_FALLBACK_LEN`] and has
/// its leading digits rotated by [`rotate_leading_digits`].
///
/// Takes the *stem*, not the file name: sanitizing `1234.ttf` would
/// otherwise yield the variant name `ttf`.
///
/// Returns an empty string when the stem yields no legal
/// identifier — the caller (the build script) warns and skips the
/// file rather than emitting source that will not compile.
pub fn fallback_sanitize(file_stem: &str) -> String {
    let tokens: Vec<String> = file_stem
        .split(|ch: char| ch.is_whitespace() || ch == '-' || ch == '_')
        .map(|token| {
            token
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
        })
        .filter(|token| !token.is_empty())
        .collect();

    let substantial: Vec<&String> = tokens
        .iter()
        .filter(|token| token.len() >= MIN_TOKEN_LEN)
        .collect();
    let kept: Vec<&String> = if substantial.is_empty() {
        tokens.iter().collect()
    } else {
        substantial
    };

    let mut out = String::new();
    match kept.first() {
        None => {}
        Some(first) if first.len() >= SELF_SUFFICIENT_TOKEN_LEN => {
            out = capitalize_first(first);
            out.truncate(MAX_FALLBACK_LEN);
        }
        Some(_) => {
            for token in kept {
                let word = capitalize_first(token);
                if out.len() + word.len() > MAX_FALLBACK_LEN {
                    break;
                }
                out.push_str(&word);
            }
        }
    }

    let rotated = rotate_leading_digits(&out);
    if rotated.chars().all(|ch| ch.is_ascii_digit()) {
        // All-digit stems ("2000.ttf") cannot become an identifier.
        return String::new();
    }
    rotated
}

/// Whether a decoded `name`-table record carries enough to build a
/// variant name from.
///
/// A font's name table holds one record per (platform, encoding,
/// language) triple, and not all of them survive the ASCII
/// reduction — a Japanese-language `FULL_NAME` record reduces to
/// nothing. The build script walks the records in table order and
/// takes the first this accepts, so a font that carries a usable
/// name anywhere in its table is named from it rather than from its
/// file name.
pub fn is_usable_font_name(decoded: &str) -> bool {
    camel_case(&ascii_font_name(decoded)).is_some()
}

/// The full derivation: prefer the font's own `name`-table entry,
/// fall back to the file stem.
///
/// `font_full_name` is the already-decoded `FULL_NAME` record (see
/// [`decode_name_record`]) or `None` when the file has no readable
/// one. An empty return means neither source yielded a legal
/// identifier.
pub fn variant_name(font_full_name: Option<&str>, file_stem: &str) -> String {
    font_full_name
        .map(ascii_font_name)
        .and_then(|ascii| camel_case(&ascii))
        .unwrap_or_else(|| fallback_sanitize(file_stem))
}

/// One font file discovered by the build script's directory walk,
/// with every naming decision already applied.
///
/// Constructed by [`FontCandidate::new`]; consumed by
/// [`select_font_variants`], which resolves collisions and fixes the
/// emission order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FontCandidate {
    /// Rust enum-variant identifier for this font. Assigned by
    /// [`variant_name`] and possibly given a numeric suffix by
    /// [`select_font_variants`] when two files derive the same name.
    pub variant: String,
    /// Human-readable family name for the generated doc comment —
    /// the font's own `FULL_NAME` when it had a readable one, the
    /// file stem otherwise.
    pub display_name: String,
    /// Absolute, forward-slash path handed to `include_bytes!`.
    pub absolute_path: String,
    /// Path relative to the font root, for the generated doc
    /// comment. Also the tiebreaker for equal-preference
    /// collisions, so it must be stable across machines — which the
    /// absolute path is not.
    pub relative_path: String,
    /// Lowercased file-stem prefix; see [`family_key`].
    pub family_key: String,
    /// Lowercased container extension; see [`is_font_extension`].
    pub extension: String,
}

impl FontCandidate {
    /// Build a candidate from the pieces a directory walk yields.
    ///
    /// `font_full_name` is the decoded `FULL_NAME` record or `None`.
    /// `relative_path` is relative to the font root and uses forward
    /// slashes on every platform. The derived variant name may be
    /// empty when neither the name table nor the stem yields a legal
    /// identifier; [`select_font_variants`] skips those with a
    /// warning.
    pub fn new(
        absolute_path: String,
        relative_path: String,
        file_stem: &str,
        extension: String,
        font_full_name: Option<&str>,
    ) -> Self {
        let ascii_name = font_full_name.map(ascii_font_name).unwrap_or_default();
        let display_name = if ascii_name.is_empty() {
            file_stem.to_string()
        } else {
            ascii_name.clone()
        };
        FontCandidate {
            variant: variant_name(font_full_name, file_stem),
            display_name,
            absolute_path,
            relative_path,
            family_key: family_key(file_stem),
            extension,
        }
    }

    /// Collision-preference key: TTF before OTF, then the
    /// lexicographically smallest relative path. Total over any set
    /// of distinct files, which is what makes selection independent
    /// of directory-walk order.
    fn preference(&self) -> (usize, &str) {
        (extension_rank(&self.extension), self.relative_path.as_str())
    }
}

/// Outcome of [`select_font_variants`]: the fonts that become
/// `AppFont` variants, plus the human-readable notes the build
/// script re-emits as `cargo:warning` lines.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FontSelection {
    /// Surviving fonts, sorted by variant name. This is the emission
    /// order of the generated enum.
    pub fonts: Vec<FontCandidate>,
    /// Notes about files that were renamed or skipped. Sorted with
    /// the decisions that produced them, so the set of warnings is
    /// as reproducible as the generated source.
    pub warnings: Vec<String>,
}

/// Resolve the discovered font files into the exact variant list the
/// generated enum will carry.
///
/// Three passes, each with an explicit comparator so the result
/// depends on the input *set* and never on its order:
///
/// 1. **Unnamable files are dropped** with a warning — a file whose
///    name table and stem both yield nothing would otherwise emit an
///    empty variant and fail the crate build.
/// 2. **One variant per family.** Font hosts ship the same face as
///    both `.ttf` and `.otf`; [`family_key`] groups them and
///    [`FontCandidate::preference`] picks the survivor. Silent —
///    this is the expected shape of the tree, not a problem.
/// 3. **Distinct fonts that derive the same name are renamed**, not
///    dropped: the preferred one keeps the bare name and the rest
///    take the first free `Name2`, `Name3`, … suffix, each with a
///    warning. Dropping them would break the documented "drop a
///    font file in and the variant appears" workflow. The
///    [`ANY_VARIANT`] sentinel is reserved up front, so a font
///    genuinely named "Any" is renamed rather than colliding with
///    it.
///
/// Costs: O(n log n) in the number of font files, a handful of
/// `String` clones per file. Runs once per build-script execution.
pub fn select_font_variants(candidates: Vec<FontCandidate>) -> FontSelection {
    let mut selection = FontSelection::default();

    let mut named: Vec<FontCandidate> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.variant.is_empty() {
            selection.warnings.push(format!(
                "font '{}' yields no legal variant name from either its name table or its \
                 file name; skipping it",
                candidate.relative_path
            ));
        } else {
            named.push(candidate);
        }
    }
    // Sorting the skip warnings separately keeps them ordered even
    // though the walk that produced them is not.
    selection.warnings.sort();

    let mut by_family: std::collections::BTreeMap<String, FontCandidate> = std::collections::BTreeMap::new();
    for candidate in named {
        match by_family.get(&candidate.family_key) {
            Some(incumbent) if incumbent.preference() <= candidate.preference() => {}
            _ => {
                by_family.insert(candidate.family_key.clone(), candidate);
            }
        }
    }

    let mut survivors: Vec<FontCandidate> = by_family.into_values().collect();
    survivors.sort_by(|left, right| {
        left.variant
            .cmp(&right.variant)
            .then_with(|| left.preference().cmp(&right.preference()))
    });

    let mut taken: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    taken.insert(ANY_VARIANT.to_string());
    for candidate in survivors.iter_mut() {
        if taken.insert(candidate.variant.clone()) {
            continue;
        }
        let base = candidate.variant.clone();
        let mut ordinal = 2usize;
        let renamed = loop {
            let attempt = format!("{base}{ordinal}");
            if taken.insert(attempt.clone()) {
                break attempt;
            }
            ordinal += 1;
        };
        selection.warnings.push(format!(
            "font '{}' derives the variant name '{}', which is already taken; emitting it as \
             '{}' instead",
            candidate.relative_path, base, renamed
        ));
        candidate.variant = renamed;
    }

    survivors.sort_by(|left, right| left.variant.cmp(&right.variant));
    selection.fonts = survivors;
    selection
}
