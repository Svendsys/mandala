// SPDX-License-Identifier: MPL-2.0

//! Reader over the workspace's own `Cargo.toml` files, so a test can
//! ask "is any dependency's version written down in more than one
//! place?" and get an answer that stays true as members are added.
//!
//! The problem this exists to solve: nothing in cargo objects when
//! two members of a workspace name the same crate at two different
//! versions. It resolves both, builds both, and the only symptom is
//! a slower build and two incompatible copies of the same types.
//! That is not hypothetical here — `strum` sat at 0.27 in the app
//! crate and 0.28 in baumhard, compiling two runtime crates and two
//! proc-macro crates, until it was unified by hand. The structural
//! fix is `[workspace.dependencies]`, and the reason to test it is
//! that the fix is only as good as the next person's remembering to
//! use it: adding `strum = "0.29"` to one member re-creates the
//! split in one line.
//!
//! So the rule from the root manifest's comment is checked here
//! rather than trusted:
//!
//! - a dependency two or more members need is declared once, in
//!   `[workspace.dependencies]`, and members write
//!   `dep.workspace = true`;
//! - a dependency in that table is not given a version by any member
//!   (which would silently override it);
//! - the table carries nothing only one member uses.
//!
//! **The member list is read, not restated.** [`member_manifests`]
//! parses `[workspace] members` out of the root manifest, so a fifth
//! crate is covered the day it is added — the failure mode
//! `test.sh`'s hand-written `-p` list demonstrated for as long as
//! `mandala_derive` existed.
//!
//! Test-only and native-only: it reads files off the filesystem, and
//! nothing in a shipped build has any business parsing manifests.
//!
//! ## On the parser
//!
//! It is line-oriented, not a TOML implementation, and it refuses
//! rather than guesses: a dependency spread across several lines
//! panics with the line that caused it instead of being silently
//! skipped. That matters more than generality here, because a parser
//! that quietly matched nothing would turn every test below into a
//! test that cannot fail. [`tests`] therefore pins the parser
//! against synthetic manifests — including ones that *must* be
//! rejected — before pointing it at the real ones.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Which dependency table a declaration was found in. The string is
/// the raw section header without its brackets, so
/// `target.'cfg(unix)'.dependencies` keeps its target predicate for
/// error messages.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Table {
    /// The root manifest's `[workspace.dependencies]` — the single
    /// source of truth, not a consumer.
    Workspace,
    /// Any `[dependencies]` / `[dev-dependencies]` /
    /// `[build-dependencies]`, plain or under a `[target.'...']`.
    Member(String),
}

/// One dependency declaration as written.
#[derive(Debug, Clone)]
pub(crate) struct Dep {
    /// The crate name to the left of the `=`.
    pub(crate) name: String,
    /// The table it was declared in.
    pub(crate) table: Table,
    /// The version string, when the declaration names one. `None`
    /// for `dep.workspace = true` and for path dependencies.
    pub(crate) version: Option<String>,
    /// Whether the declaration defers to `[workspace.dependencies]`.
    pub(crate) inherits_workspace: bool,
    /// Whether the declaration is an intra-workspace `path = ` dep.
    /// Those carry no version and are exempt from every rule here.
    pub(crate) is_path: bool,
}

/// The dependency declarations of one manifest.
#[derive(Debug, Clone)]
pub(crate) struct Manifest {
    /// Repo-relative path, used verbatim in assertion messages.
    pub(crate) path: String,
    /// Every declaration found, in file order.
    pub(crate) deps: Vec<Dep>,
}

impl Manifest {
    /// The declarations in this manifest's member tables — i.e.
    /// everything except a `[workspace.dependencies]` entry.
    pub(crate) fn member_deps(&self) -> impl Iterator<Item = &Dep> {
        self.deps.iter().filter(|d| !matches!(d.table, Table::Workspace))
    }

    /// The declarations in this manifest's `[workspace.dependencies]`
    /// table. Empty for everything but the root manifest.
    pub(crate) fn workspace_deps(&self) -> impl Iterator<Item = &Dep> {
        self.deps.iter().filter(|d| matches!(d.table, Table::Workspace))
    }
}

/// Absolute path to something under the repo root, resolved from
/// baumhard's own `CARGO_MANIFEST_DIR` the same way
/// [`crate::util::doc_fixtures`] does.
pub(crate) fn repo_path(relative: &str) -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join(relative)
}

/// Every manifest in the workspace, repo-relative, root first.
///
/// Read out of `[workspace] members` rather than listed, so this
/// cannot be the thing that goes stale. The root manifest is both the
/// workspace root and a package (`mandala`), so it appears once and
/// is parsed for both roles.
pub(crate) fn member_manifests() -> Vec<String> {
    let root =
        std::fs::read_to_string(repo_path("Cargo.toml")).expect("the root Cargo.toml must be readable");

    let members = root
        .split_once("members = [")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once(']'))
        .map(|(list, _)| list)
        .expect("the root Cargo.toml must declare `[workspace] members = [...]`");

    let mut manifests = vec!["Cargo.toml".to_string()];
    manifests.extend(
        members
            .split(',')
            .map(|entry| entry.trim().trim_matches('"').trim())
            .filter(|entry| !entry.is_empty())
            .map(|dir| format!("{dir}/Cargo.toml")),
    );
    manifests
}

/// Parse every manifest in the workspace.
pub(crate) fn workspace_manifests() -> Vec<Manifest> {
    member_manifests()
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(repo_path(&path))
                .unwrap_or_else(|e| panic!("{path} must be readable: {e}"));
            parse(&path, &text)
        })
        .collect()
}

/// Classify a section header (already stripped of its brackets) as a
/// dependency table, or `None` if it is something else entirely.
fn table_for(header: &str) -> Option<Table> {
    const KINDS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

    if header == "workspace.dependencies" {
        return Some(Table::Workspace);
    }
    if KINDS.contains(&header) {
        return Some(Table::Member(header.to_string()));
    }
    // `target.'cfg(...)'.dependencies` and its dev/build siblings.
    // Matched on the suffix so any cfg predicate is accepted, but
    // still anchored on `target.` so an unrelated future section
    // ending in `dependencies` is not swept in silently.
    if header.starts_with("target.") && KINDS.iter().any(|k| header.ends_with(&format!(".{k}"))) {
        return Some(Table::Member(header.to_string()));
    }
    None
}

/// Parse one manifest's dependency declarations.
///
/// Panics on any dependency line it cannot read with confidence,
/// naming the file and the line. Refusing is the point: a parser that
/// shrugged at an unfamiliar shape would silently shrink the set
/// every test here checks.
pub(crate) fn parse(path: &str, text: &str) -> Manifest {
    let mut deps = Vec::new();
    let mut table: Option<Table> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // `[[bench]]` and friends end a dependency table just as
            // any other header does; `table_for` returns None and the
            // lines after it are ignored.
            let header = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_string();
            table = table_for(&header);
            continue;
        }
        let Some(table) = table.clone() else { continue };

        let (lhs, rhs) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("{path}: cannot read dependency line {raw:?}"));
        deps.push(parse_dep(path, raw, lhs.trim(), rhs.trim(), table));
    }

    Manifest {
        path: path.to_string(),
        deps,
    }
}

/// Read one `name = <value>` dependency line.
fn parse_dep(path: &str, raw: &str, lhs: &str, rhs: &str, table: Table) -> Dep {
    // `serde.workspace = true` — a dotted key, which is the only
    // dotted form this workspace uses. Anything else dotted is a
    // shape the rules below were not written against, so refuse it
    // rather than record it wrong.
    let (name, dotted_key) = match lhs.split_once('.') {
        Some((name, key)) => (name.trim(), Some(key.trim())),
        None => (lhs, None),
    };
    let name = name.trim_matches('"').to_string();

    if let Some(key) = dotted_key {
        assert_eq!(
            key, "workspace",
            "{path}: {raw:?} uses a dotted dependency key this reader does not \
             understand; teach `util::manifests` about it rather than leaving it \
             unchecked"
        );
        assert_eq!(rhs, "true", "{path}: {raw:?} — `{name}.workspace` must be `true`");
        return Dep {
            name,
            table,
            version: None,
            inherits_workspace: true,
            is_path: false,
        };
    }

    // A bare version string: `rustc-hash = "2.1.2"`.
    if let Some(version) = rhs.strip_prefix('"').and_then(|r| r.split_once('"')) {
        return Dep {
            name,
            table,
            version: Some(version.0.to_string()),
            inherits_workspace: false,
            is_path: false,
        };
    }

    // An inline table. Multi-line tables are refused, not skipped —
    // see the module docs on why silence is the dangerous answer.
    let body = rhs
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .unwrap_or_else(|| {
            panic!(
                "{path}: {raw:?} is neither a bare version nor a single-line inline \
                 table. `util::manifests` reads manifests line by line; either keep \
                 the declaration on one line or teach the reader the new shape."
            )
        });

    Dep {
        name,
        table,
        version: inline_value(body, "version").map(quoted),
        inherits_workspace: inline_value(body, "workspace").is_some_and(|v| v.starts_with("true")),
        is_path: inline_value(body, "path").is_some(),
    }
}

/// The value following `key =` inside an inline table body, or `None`
/// if the key is absent.
///
/// Matches `key` only as a whole word immediately followed by `=`, so
/// a feature string that happens to contain the key's letters — or
/// the `version` inside `version-compat` — does not match.
fn inline_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(offset) = body[from..].find(key) {
        let at = from + offset;
        let boundary = match body[..at].chars().next_back() {
            None => true,
            Some(c) => !(c.is_alphanumeric() || c == '_' || c == '-'),
        };
        let after = body[at + key.len()..].trim_start();
        if boundary {
            if let Some(value) = after.strip_prefix('=') {
                return Some(value.trim());
            }
        }
        from = at + key.len();
    }
    None
}

/// Strip the surrounding quotes from a TOML string value.
fn quoted(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('"')
        .split('"')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The names declared in `[workspace.dependencies]`.
pub(crate) fn workspace_table(manifests: &[Manifest]) -> BTreeSet<String> {
    manifests
        .iter()
        .flat_map(Manifest::workspace_deps)
        .map(|d| d.name.clone())
        .collect()
}

/// For each dependency given an explicit version by more than one
/// manifest, the manifests that give it one.
///
/// This is the `strum` failure in general form: two members, two
/// version strings, no complaint from cargo. Path dependencies are
/// exempt — an intra-workspace `path = ` dep has no version to
/// diverge.
pub(crate) fn versions_declared_in_several_manifests(
    manifests: &[Manifest],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut sites: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for manifest in manifests {
        for dep in manifest.member_deps() {
            if dep.is_path || dep.version.is_none() {
                continue;
            }
            sites
                .entry(dep.name.clone())
                .or_default()
                .insert(manifest.path.clone());
        }
    }
    sites.retain(|_, where_| where_.len() > 1);
    sites
}

/// For each `[workspace.dependencies]` entry a member shadows with
/// its own version, the manifests that shadow it.
///
/// Cargo takes the member's word, so an override turns the shared
/// table into decoration — the same split as before, one indirection
/// further away from the reader.
pub(crate) fn workspace_entries_overridden(manifests: &[Manifest]) -> BTreeMap<String, BTreeSet<String>> {
    let shared = workspace_table(manifests);
    let mut overrides: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for manifest in manifests {
        for dep in manifest.member_deps() {
            if dep.version.is_some() && shared.contains(&dep.name) {
                overrides
                    .entry(dep.name.clone())
                    .or_default()
                    .insert(manifest.path.clone());
            }
        }
    }
    overrides
}

/// For each `[workspace.dependencies]` entry, the manifests that
/// actually inherit it.
pub(crate) fn workspace_entry_consumers(manifests: &[Manifest]) -> BTreeMap<String, BTreeSet<String>> {
    let mut consumers: BTreeMap<String, BTreeSet<String>> = workspace_table(manifests)
        .into_iter()
        .map(|name| (name, BTreeSet::new()))
        .collect();
    for manifest in manifests {
        for dep in manifest.member_deps() {
            if !dep.inherits_workspace {
                continue;
            }
            consumers
                .entry(dep.name.clone())
                .or_default()
                .insert(manifest.path.clone());
        }
    }
    consumers
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest pair in the shape this workspace uses, for the
    /// parser and detector pins below. Kept synthetic so the negative
    /// controls can express failures the real manifests must not
    /// have.
    fn fixture(path: &str, body: &str) -> Manifest {
        parse(path, body)
    }

    /// The parser has to read every shape the real manifests contain,
    /// because a shape it skips is a dependency the rules below never
    /// see. Each line here appears in the workspace for real.
    #[test]
    fn test_parser_reads_the_shapes_this_workspace_uses() {
        let m = fixture(
            "fixture/Cargo.toml",
            r#"
[package]
name = "fixture"
autobenches = false

[workspace.dependencies]
strum = "0.28.0"
serde = { version = "1.0.228", features = ["derive"] }

[dependencies]
# a comment, and a blank line, both ignored

strum.workspace = true
syn = { workspace = true, features = ["full"] }
baumhard = {path="./lib/baumhard"}
wgpu = { version = "29.0", features = ["webgl"] }

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
arboard = "3.6.1"

[[bench]]
name = "test_bench"
harness = false
"#,
        );

        let by_name = |name: &str| -> Dep {
            m.deps
                .iter()
                .find(|d| d.name == name && !matches!(d.table, Table::Workspace))
                .unwrap_or_else(|| panic!("{name} must be parsed out of the member tables"))
                .clone()
        };

        assert_eq!(
            workspace_table(std::slice::from_ref(&m)),
            ["serde".to_string(), "strum".to_string()].into(),
            "`[workspace.dependencies]` entries must be recognized as the table, not \
             as member declarations"
        );

        let strum = by_name("strum");
        assert!(strum.inherits_workspace && strum.version.is_none());

        let syn = by_name("syn");
        assert!(
            syn.inherits_workspace && syn.version.is_none(),
            "`{{ workspace = true, features = [...] }}` inherits; the added features \
             are not an override"
        );

        assert!(by_name("baumhard").is_path, "a path dep has no version");
        assert_eq!(by_name("wgpu").version.as_deref(), Some("29.0"));

        let arboard = by_name("arboard");
        assert_eq!(arboard.version.as_deref(), Some("3.6.1"));
        assert_eq!(
            arboard.table,
            Table::Member("target.'cfg(not(target_arch = \"wasm32\"))'.dependencies".into()),
            "a target-gated table is still a member table"
        );

        assert!(
            !m.deps.iter().any(|d| d.name == "name" || d.name == "harness"),
            "`[[bench]]` and `[package]` keys must not be mistaken for dependencies"
        );
    }

    /// Refusing beats guessing: a multi-line declaration this reader
    /// cannot follow must stop the test run, not vanish from the set
    /// being checked.
    #[test]
    #[should_panic(expected = "single-line inline")]
    fn test_parser_refuses_a_multi_line_dependency() {
        parse(
            "fixture/Cargo.toml",
            "[dependencies]\nweb-sys = { version = \"0.3\", features = [\n  \"Window\",\n] }\n",
        );
    }

    /// The negative control for the headline rule. Without it, every
    /// assertion over the real manifests could be passing because the
    /// detector never reports anything.
    #[test]
    fn test_detector_catches_a_version_split_across_manifests() {
        let split = [
            fixture("a/Cargo.toml", "[dependencies]\nstrum = \"0.27\"\n"),
            fixture("b/Cargo.toml", "[dependencies]\nstrum = \"0.28.0\"\n"),
        ];
        let found = versions_declared_in_several_manifests(&split);
        assert_eq!(
            found.keys().collect::<Vec<_>>(),
            vec!["strum"],
            "a crate versioned in two manifests is exactly what this must report"
        );

        let unified = [
            fixture(
                "a/Cargo.toml",
                "[workspace.dependencies]\nstrum = \"0.28.0\"\n\n[dependencies]\nstrum.workspace = true\n",
            ),
            fixture("b/Cargo.toml", "[dependencies]\nstrum.workspace = true\n"),
        ];
        assert!(
            versions_declared_in_several_manifests(&unified).is_empty(),
            "the same crate inherited from the shared table is not a split"
        );
    }

    /// The other negative control: hoisting is decorative if a member
    /// may still name its own version, since cargo takes the member's.
    #[test]
    fn test_detector_catches_a_shadowed_workspace_entry() {
        let shadowed = [
            fixture(
                "a/Cargo.toml",
                "[workspace.dependencies]\nstrum = \"0.28.0\"\n\n[dependencies]\nstrum = \"0.27\"\n",
            ),
            fixture("b/Cargo.toml", "[dependencies]\nstrum.workspace = true\n"),
        ];
        assert_eq!(
            workspace_entries_overridden(&shadowed).keys().collect::<Vec<_>>(),
            vec!["strum"]
        );
    }

    /// The member list is read out of `[workspace] members`, so this
    /// pins that the read worked rather than pinning the membership
    /// itself: a fifth crate should extend the checks below on the
    /// day it is added, not fail here.
    #[test]
    fn test_workspace_member_list_is_read_from_the_root_manifest() {
        let manifests = member_manifests();
        assert_eq!(
            manifests.first().map(String::as_str),
            Some("Cargo.toml"),
            "the root manifest is a package too and must be checked as one"
        );
        for expected in [
            "lib/baumhard/Cargo.toml",
            "lib/mandala_derive/Cargo.toml",
            "crates/maptool/Cargo.toml",
        ] {
            assert!(
                manifests.iter().any(|m| m == expected),
                "`[workspace] members` must have yielded {expected}; got {manifests:?}"
            );
        }
    }

    /// A parse that found nothing would make every rule below vacuous.
    /// Pinned against the shared table specifically, since that is the
    /// structure the rules are about.
    #[test]
    fn test_the_real_manifests_parse_into_something_worth_checking() {
        let manifests = workspace_manifests();
        let declared: usize = manifests.iter().map(|m| m.member_deps().count()).sum();
        assert!(
            declared > 30,
            "only {declared} dependency declarations were read out of the workspace; \
             `util::manifests` has stopped understanding the manifests and the checks \
             below are no longer checking anything"
        );

        let shared = workspace_table(&manifests);
        assert!(
            shared.contains("strum"),
            "`strum` — the crate whose 0.27/0.28 split motivated \
             `[workspace.dependencies]` — must be declared there; found {shared:?}"
        );
    }

    /// The headline rule: one version, written once.
    #[test]
    fn test_no_dependency_version_is_written_in_two_manifests() {
        let manifests = workspace_manifests();
        let split = versions_declared_in_several_manifests(&manifests);
        assert!(
            split.is_empty(),
            "these dependencies name a version in more than one manifest, which is \
             how `strum` ended up at 0.27 and 0.28 at the same time: {split:#?}. Move \
             each to `[workspace.dependencies]` in the root Cargo.toml and write \
             `dep.workspace = true` at every use site — per-crate `features` still \
             work, they are additive on top of the shared entry."
        );
    }

    /// ...and not un-written by a member that names its own.
    #[test]
    fn test_workspace_dependencies_are_not_shadowed_by_members() {
        let manifests = workspace_manifests();
        let overridden = workspace_entries_overridden(&manifests);
        assert!(
            overridden.is_empty(),
            "these members give a version to a crate `[workspace.dependencies]` \
             already versions, which cargo resolves in the member's favor and leaves \
             the shared entry as decoration: {overridden:#?}. Write \
             `dep.workspace = true` instead; add `features` beside it if the crate \
             needs more than the shared entry declares."
        );
    }

    /// The table holds the shared set and nothing else. A single-user
    /// entry separates a version from its one use site, which is a
    /// lookup for no invariant.
    #[test]
    fn test_workspace_table_carries_only_shared_dependencies() {
        let manifests = workspace_manifests();
        let consumers = workspace_entry_consumers(&manifests);
        let shared = workspace_table(&manifests);

        let solo: BTreeMap<_, _> = consumers
            .iter()
            .filter(|(name, who)| shared.contains(*name) && who.len() < 2)
            .collect();
        assert!(
            solo.is_empty(),
            "`[workspace.dependencies]` should hold exactly the crates two or more \
             members need; these have fewer: {solo:#?}. Move each back into the one \
             manifest that uses it — or, if a second member has just stopped using \
             it, that is the change to notice here."
        );
    }

    /// Every `dep.workspace = true` names a real entry. Cargo would
    /// also refuse this, but reaching it through the reader proves the
    /// reader is seeing both halves of the relationship rather than
    /// agreeing with itself.
    #[test]
    fn test_every_workspace_inheritance_resolves() {
        let manifests = workspace_manifests();
        let shared = workspace_table(&manifests);
        let unresolved: BTreeMap<_, _> = workspace_entry_consumers(&manifests)
            .into_iter()
            .filter(|(name, _)| !shared.contains(name))
            .collect();
        assert!(
            unresolved.is_empty(),
            "these members inherit a dependency the root \
             `[workspace.dependencies]` table does not declare: {unresolved:#?}"
        );
    }
}
