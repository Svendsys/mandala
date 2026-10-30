// SPDX-License-Identifier: MPL-2.0

//! Readers over the workspace's **own source text**, for the tests
//! whose subject is the repository rather than a value a function
//! returned.
//!
//! Three such tests existed when this was written, and each of them
//! used to carry its own answer to the same question — *which of
//! these bytes are shipped code?*
//!
//! - `mindmap::loader::tests::test_every_mindmap_write_goes_through_the_contract`
//!   asks whether any path outside `loader::to_json_value` turns a
//!   `MindMap` into bytes.
//! - [`crate::util::serde_coverage::TypeGraph`] asks which types a
//!   deserializer can be handed.
//! - [`tests::test_no_doc_cites_a_test_that_does_not_exist`] asks
//!   whether every `test_*` name a comment or a spec writes down is
//!   one the sources declare.
//!
//! Three answers meant three chances to be wrong, and two of them
//! were. The loader's split the file at the first `#[cfg(test)]` and
//! kept the text before it, on the premise that a test module hangs
//! off the *end* of a file. A `mod.rs` declares
//! `#[cfg(test)] mod tests;` among its module *list* instead, so
//! everything the file goes on to say is below the gate:
//! `src/application/document/mod.rs` was read to line 35 of 757, and
//! a raw `serde_json::to_string_pretty(&map)` planted at line 79
//! passed the guard that exists to forbid exactly that. The type
//! graph consulted a `cfg` reader for inline `mod x { … }` and
//! nothing at all for the out-of-line `mod x;` form, so eleven
//! test-only types sat in an index that promises to hold none.
//!
//! So the question is answered once, here. [`is_test_gated`] is the
//! only reader of a `cfg` predicate in the crate;
//! [`production_source`] is the only excision of test code from a
//! file's text; [`test_gated_module_files`] is the only resolution of
//! a test-gated `mod x;` to the files it names.
//!
//! A fourth asker arrived later and is what that consolidation buys:
//! [`crate::util::unwrap_posture`] asks whether any shipped line
//! calls `unwrap()`, and answers "which bytes ship" by calling
//! [`production_source`] rather than by deriving a fourth answer of
//! its own. It did pay for one correction here — see that function
//! on the `#![cfg(test)]` *inner*-attribute form, which gates a whole
//! file and which nothing here had been reading. A fifth,
//! [`crate::util::doc_identifiers`], asks whether every `snake_case`
//! identifier a doc comment writes in backticks still exists, and
//! reads the sources through [`workspace_rust_sources`] and the item
//! walk's attribute helpers here.
//!
//! **Both excisions fail toward noise.** [`production_source`] does
//! not descend into function bodies, so a `#[cfg(test)] mod` written
//! inside an `fn` stays in the text a caller scans; the consequence is
//! a false positive a reader adjudicates, never a shipped path that
//! goes unlooked-at. A shape the readers here cannot make sense of —
//! a `#[path = "…"]` module, a file a `mod` names that is not on
//! disk — stops the run by name rather than dropping out of the
//! checked set, which is the discipline [`crate::util::manifests`]
//! records for the manifest parser and for the same reason.
//!
//! Test-only and native-only: it reads `.rs` and `.md` files off the
//! filesystem and leans on `syn`, a dev-dependency.

use crate::util::doc_fixtures::repo_path;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syn::spanned::Spanned;
use syn::{Attribute, ImplItem, Item, TraitItem};

/// Repo-relative directories that hold the workspace's Rust sources.
///
/// Named rather than discovered by walking the repository root,
/// because the root of a checkout is not reliably the root of *one*
/// checkout: an agent worktree under `.claude/worktrees/` is a second
/// full copy of this repository, and a walk that started at the top
/// would read another branch's files as though they were this one's.
pub(crate) const RUST_ROOTS: &[&str] = &["lib", "src", "crates"];

/// Repo-relative directories that hold Markdown this workspace owns,
/// alongside the specs and conventions files sitting at the root.
/// `vendor/` is deliberately absent — vendored agent skills are
/// somebody else's prose and none of this crate's business.
pub(crate) const MARKDOWN_ROOTS: &[&str] = &["format", "work_plans", "lib", "crates", "src", "maps"];

/// Repo-relative directories that hold shell this workspace runs,
/// alongside the scripts sitting at the root. `vendor/` is absent for
/// the same reason it is absent from [`MARKDOWN_ROOTS`].
pub(crate) const SHELL_ROOTS: &[&str] = &["scripts"];

/// Repo-relative directory holding the CI workflows — the other
/// place, besides the shell scripts, where this repository *runs* a
/// command rather than describing one.
pub(crate) const WORKFLOW_ROOT: &str = ".github/workflows";

/// Directory names no walk here descends into: build output, vendored
/// third-party trees, git's own storage, and the browser bundle.
const SKIPPED_DIRS: &[&str] = &["target", "vendor", ".git", "dist", "node_modules"];

/// Whether `attrs` compile the item only under `cfg(test)`.
///
/// `all(...)` is descended into and `any(...)` / `not(...)`
/// deliberately are not: `#[cfg(all(test, not(target_arch =
/// "wasm32")))]` — the spelling this crate actually uses for its
/// native-only test modules — gates on `test`, while
/// `#[cfg(any(test, feature = "x"))]` does not and `#[cfg(not(test))]`
/// gates on its absence. Reading the bare `test` ident alone missed
/// the `all(...)` form, which is the same list-form blind spot
/// [`skip_unmodeled_option`] closes on the serde side, with the same
/// consequence: a test-only declaration was treated as part of the
/// on-disk model.
pub(crate) fn is_test_gated(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") && cfg_predicate_requires_test(attr))
}

/// `true` when the `cfg` predicate written in `attr`, or in an
/// `all(...)` nested to any depth inside it, names the bare `test`
/// flag.
fn cfg_predicate_requires_test(attr: &Attribute) -> bool {
    let mut gated = false;
    let _ = attr.parse_nested_meta(|meta| cfg_arm_requires_test(&meta, &mut gated));
    gated
}

/// One arm of a `cfg` predicate. `all(...)` conjoins, so descending
/// into it preserves the meaning; every other shape is consumed
/// whole so the arms after it are still read.
fn cfg_arm_requires_test(meta: &syn::meta::ParseNestedMeta, gated: &mut bool) -> syn::Result<()> {
    if meta.path.is_ident("test") {
        *gated = true;
    } else if meta.path.is_ident("all") {
        meta.parse_nested_meta(|nested| cfg_arm_requires_test(&nested, gated))?;
    } else {
        skip_unmodeled_option(meta)?;
    }
    Ok(())
}

/// Consume whatever follows an attribute option the caller does not
/// model, so the options written **after** it in the same attribute
/// are still seen.
///
/// Rust's attribute grammar — the one `cfg` and `serde` are both
/// written in — has three shapes: a bare flag
/// (`deny_unknown_fields`, `test`), a `name = value` pair
/// (`tag = "kind"`, `target_arch = "wasm32"`), and a nested list
/// (`rename_all(deserialize = "camelCase")`, `bound(...)`,
/// `all(...)`). The list form is the one that used to end the parse —
/// `parse_nested_meta` hands over the path, finds `(` where it wanted
/// `,`, and errors; the caller discards the error, and every option
/// after it in that attribute is never read.
/// `#[serde(rename_all(deserialize = "camelCase"), deny_unknown_fields)]`
/// therefore read as carrying no `deny_unknown_fields` at all, which
/// is a *silent* restoration of reject-at-load.
///
/// The list arm recurses through syn's own nested-meta parser rather
/// than matching the spellings serde has today: the next option serde
/// grows would re-open the hole, and the point of the walk in
/// [`crate::util::serde_coverage`] is that nothing is listed.
///
/// Cost: one token-tree walk of the skipped option; no allocation
/// beyond what syn's parser needs.
pub(crate) fn skip_unmodeled_option(meta: &syn::meta::ParseNestedMeta) -> syn::Result<()> {
    if meta.input.peek(syn::Token![=]) {
        meta.value()?.parse::<syn::Expr>()?;
    } else if meta.input.peek(syn::token::Paren) {
        meta.parse_nested_meta(|nested| skip_unmodeled_option(&nested))?;
    }
    Ok(())
}

/// `text` with every `#[cfg(test)]`-gated item replaced by blank
/// lines — the file as a shipped build sees it, **at its original
/// line numbers**.
///
/// Line numbering is the reason this returns a string of the same
/// height rather than a filtered list: a caller reporting an offender
/// has to be able to say `file:line`, and a line number that shifted
/// with the excision would point at the wrong code.
///
/// What it descends into: inline `mod x { … }`, `impl` blocks, and
/// `trait` bodies. What it does not: the inside of a function. A
/// `#[cfg(test)] mod` written in a function body therefore survives
/// into the returned text, and that is the safe direction — the
/// caller reports a test as though it were a shipped path, which a
/// reader adjudicates, rather than skipping a shipped path in
/// silence.
///
/// **A file can also gate itself.** `#![cfg(test)]` written as an
/// *inner* attribute makes the whole file test source, and three
/// files in this workspace are written that way:
/// `src/application/macros/tests.rs`,
/// `…/cross_dispatch/selection/tests.rs` and
/// `crates/maptool/src/verify/test_helpers.rs`. The whole file is
/// blanked when its own attributes gate on `test`.
///
/// Only two of the three need that, and saying "three" was the
/// original claim here. `macros/tests.rs` and `selection/tests.rs`
/// are declared by a bare `mod tests;` carrying no attribute, so
/// neither the item walk below nor [`test_gated_module_files`] sees
/// a gate anywhere and the inner attribute is the only one there is.
/// `test_helpers.rs` is declared `#[cfg(test)] pub(crate) mod
/// test_helpers;` (in `crates/maptool/src/verify/mod.rs`), which
/// [`test_gated_module_files`] already resolves — this reader never
/// sees the file, and blanking it is redundant rather than
/// load-bearing.
///
/// Reading only the *item* attributes therefore returned every line
/// of the first two as shipped code, and the first scan built on
/// this reader collected twenty-six of their `unwrap()`s as
/// production offenders — nineteen from `macros/tests.rs` and seven
/// from `selection/tests.rs`, none from `test_helpers.rs`, which is
/// how the two-of-three split shows up in the numbers.
///
/// Panics when `text` is not valid Rust, naming `file`: every caller
/// is a guard test whose whole job is to notice that the source
/// moved, and a file that silently produced no production text would
/// pass any scan built on it.
///
/// Cost: one `syn` parse of the file plus one pass over its lines.
pub(crate) fn production_source(file: &Path, text: &str) -> String {
    let parsed =
        syn::parse_file(text).unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
    let mut blanked = vec![false; text.lines().count()];
    if is_test_gated(&parsed.attrs) {
        blanked.fill(true);
    }
    blank_test_gated_items(&parsed.items, &mut blanked);
    let kept: Vec<&str> = text
        .lines()
        .enumerate()
        .map(|(index, line)| if blanked[index] { "" } else { line })
        .collect();
    kept.join("\n")
}

/// Mark every line covered by a test-gated item, descending through
/// the item forms that can carry more items.
fn blank_test_gated_items(items: &[Item], blanked: &mut [bool]) {
    for item in items {
        if is_test_gated(item_attrs(item)) {
            blank_span(item.span(), blanked);
            continue;
        }
        match item {
            Item::Mod(item) => {
                if let Some((_, inner)) = &item.content {
                    blank_test_gated_items(inner, blanked);
                }
            }
            Item::Impl(item) => {
                for inner in &item.items {
                    if is_test_gated(impl_item_attrs(inner)) {
                        blank_span(inner.span(), blanked);
                    }
                }
            }
            Item::Trait(item) => {
                for inner in &item.items {
                    if is_test_gated(trait_item_attrs(inner)) {
                        blank_span(inner.span(), blanked);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Mark the lines `span` covers. A span whose line is outside the
/// file — which `proc-macro2` produces for tokens it synthesized
/// rather than read — is dropped rather than clamped: blanking an
/// arbitrary line would be worse than blanking none.
fn blank_span(span: proc_macro2::Span, blanked: &mut [bool]) {
    for line in span.start().line..=span.end().line {
        if let Some(slot) = line.checked_sub(1).and_then(|index| blanked.get_mut(index)) {
            *slot = true;
        }
    }
}

/// The attributes of any `Item` form that carries them. `Verbatim`
/// is tokens syn declined to parse and has none.
pub(crate) fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

/// See [`item_attrs`]; the same for the members of an `impl` block.
pub(crate) fn impl_item_attrs(item: &ImplItem) -> &[Attribute] {
    match item {
        ImplItem::Const(item) => &item.attrs,
        ImplItem::Fn(item) => &item.attrs,
        ImplItem::Type(item) => &item.attrs,
        ImplItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

/// See [`item_attrs`]; the same for the members of a `trait` body.
pub(crate) fn trait_item_attrs(item: &TraitItem) -> &[Attribute] {
    match item {
        TraitItem::Const(item) => &item.attrs,
        TraitItem::Fn(item) => &item.attrs,
        TraitItem::Type(item) => &item.attrs,
        TraitItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

/// Every file reachable from a `#[cfg(test)]`-gated **out-of-line**
/// module declaration in `files`, transitively.
///
/// The half of "test sources are excluded" that a filesystem walk
/// cannot see. `#[cfg(test)] mod tests { … }` is an attribute on an
/// item the parser is holding; `#[cfg(test)] mod tests;` is an
/// attribute on an item whose body is *another file*, and a walk that
/// visits files has no idea the declaration exists. `util/mod.rs`
/// gates five modules that way — `bench_surface`, `manifests`,
/// `serde_coverage`, `serde_probe` and `source_scan` — and the three
/// that carried the shape when this was written (`manifests`,
/// `serde_coverage`, and `test_logger`, since widened to `pub` for
/// callers in the `mandala` crate) were all indexed as model
/// sources: eleven test-only types in a graph whose contract is that
/// it holds none.
///
/// Transitive because a gated module can declare further modules of
/// its own, and those are test sources for the same reason their
/// parent is.
///
/// Panics on a `mod` this resolver cannot follow — see
/// [`module_file`].
///
/// Cost: one read and `syn` parse per file in `files`, plus one per
/// file pulled into the closure.
pub(crate) fn test_gated_module_files(files: &[PathBuf]) -> BTreeSet<PathBuf> {
    let mut excluded = BTreeSet::new();
    let mut frontier: Vec<PathBuf> = Vec::new();
    for file in files {
        for (path, gated) in declared_modules(file) {
            if gated && excluded.insert(path.clone()) {
                frontier.push(path);
            }
        }
    }
    while let Some(file) = frontier.pop() {
        for (path, _) in declared_modules(&file) {
            if excluded.insert(path.clone()) {
                frontier.push(path);
            }
        }
    }
    excluded
}

/// Every out-of-line `mod x;` declared by `file`, as `(the file it
/// names, whether the declaration is test-gated)`. Inline
/// `mod x { … }` blocks are descended into, since a module declared
/// inside one resolves against a directory of its own.
fn declared_modules(file: &Path) -> Vec<(PathBuf, bool)> {
    let text =
        std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
    let parsed =
        syn::parse_file(&text).unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
    let mut out = Vec::new();
    collect_declared_modules(&parsed.items, file, &module_dir(file), false, &mut out);
    out
}

/// The directory a file's `mod x;` declarations resolve against:
/// `foo/mod.rs`, `lib.rs` and `main.rs` own the directory they sit
/// in, every other `foo.rs` owns `foo/`.
fn module_dir(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new("")).to_path_buf();
    let stem = file
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    if stem == "mod" || stem == "lib" || stem == "main" {
        parent
    } else {
        parent.join(stem)
    }
}

fn collect_declared_modules(
    items: &[Item],
    file: &Path,
    dir: &Path,
    inherited_gate: bool,
    out: &mut Vec<(PathBuf, bool)>,
) {
    for item in items {
        let Item::Mod(item) = item else { continue };
        if item.attrs.iter().any(|attr| attr.path().is_ident("path")) {
            panic!(
                "{}: `mod {}` carries a `#[path = \"…\"]` attribute, which this resolver does \
                 not follow. Teach it the shape rather than letting the module drop out of \
                 the checked set.",
                file.display(),
                item.ident
            );
        }
        let gated = inherited_gate || is_test_gated(&item.attrs);
        match &item.content {
            Some((_, inner)) => {
                collect_declared_modules(inner, file, &dir.join(item.ident.to_string()), gated, out);
            }
            None => out.push((module_file(file, dir, &item.ident.to_string()), gated)),
        }
    }
}

/// The file `mod name;` names, resolved against `dir`: `dir/name.rs`
/// or `dir/name/mod.rs`.
///
/// Panics when neither exists or both do. Neither is a module
/// declaration nobody can follow; both is ambiguous to rustc as well.
/// A resolver that returned `None` here would quietly stop excluding
/// the module and its whole subtree, which is the failure this exists
/// to prevent.
fn module_file(declared_in: &Path, dir: &Path, name: &str) -> PathBuf {
    let flat = dir.join(format!("{name}.rs"));
    let nested = dir.join(name).join("mod.rs");
    match (flat.is_file(), nested.is_file()) {
        (true, false) => flat,
        (false, true) => nested,
        (found_flat, _) => panic!(
            "{}: `mod {name};` resolves to {} candidate file(s) — {} and {} — and this \
             resolver needs exactly one.",
            declared_in.display(),
            usize::from(found_flat) + usize::from(nested.is_file()),
            flat.display(),
            nested.display()
        ),
    }
}

/// Every `.rs` file under [`RUST_ROOTS`], in sorted order.
pub(crate) fn workspace_rust_sources() -> Vec<PathBuf> {
    collect_by_extension(RUST_ROOTS, ".rs", false)
}

/// Every `.md` file under [`MARKDOWN_ROOTS`] plus the ones at the
/// repository root — `CLAUDE.md`, `CODE_CONVENTIONS.md`,
/// `TEST_CONVENTIONS.md`, `CONCEPTS.md`, `AGENTS.md`, `README.md`.
pub(crate) fn workspace_markdown_docs() -> Vec<PathBuf> {
    collect_by_extension(MARKDOWN_ROOTS, ".md", true)
}

/// Every `.sh` file under [`SHELL_ROOTS`] plus the ones at the
/// repository root — `test.sh`, `bench.sh`, `build.sh`, `run.sh`,
/// `debug_build.sh`.
///
/// The scripts are source too, and the one that matters most here is
/// `test.sh`: it is where the commands the documents describe are
/// actually run, so a rule checked in the prose and not in the script
/// is checked in the cheaper of the two places.
pub(crate) fn workspace_shell_scripts() -> Vec<PathBuf> {
    collect_by_extension(SHELL_ROOTS, ".sh", true)
}

/// Every workflow file under [`WORKFLOW_ROOT`] — `test.yml` and
/// `license-headers.yml` today.
///
/// CI runs commands, so it states rules about how this repository is
/// built in exactly the way the scripts do. It runs `./test.sh --lint`
/// today and no `cargo` line of its own, which is the reason to read
/// it now rather than after the first `cargo` line is added to a step
/// and quietly escapes a scan that stopped at the file extension.
pub(crate) fn workspace_ci_workflows() -> Vec<PathBuf> {
    let mut out = collect_by_extension(&[WORKFLOW_ROOT], ".yml", false);
    out.extend(collect_by_extension(&[WORKFLOW_ROOT], ".yaml", false));
    out.sort();
    out
}

fn collect_by_extension(roots: &[&str], extension: &str, include_repo_root: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        collect_recursively(&repo_path(root), extension, &mut out);
    }
    if include_repo_root {
        let root = repo_path("");
        let entries =
            std::fs::read_dir(&root).unwrap_or_else(|e| panic!("{} must be readable: {e}", root.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_file() && path.to_string_lossy().ends_with(extension) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn collect_recursively(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                collect_recursively(&path, extension, out);
            }
        } else if name.ends_with(extension) {
            out.push(path);
        }
    }
}

/// What [`scan_test_citations`] found: the citations that resolve to
/// nothing, and how many resolved per file.
pub(crate) struct CitationScan {
    /// One rendered `"<file>:<line>: <name>"` per citation that names
    /// no declaration in the sources.
    pub unresolved: Vec<String>,
    /// Resolved citations per repo-relative file, so a caller can
    /// notice that the walk stopped looking somewhere.
    pub resolved: BTreeMap<String, usize>,
}

/// Every `test_*` name written in a `//` comment or a Markdown file,
/// checked against the names the sources declare.
///
/// A citation is one of two things, decided by the name itself:
///
/// - **a family**, when it ends in `_` — the trailing-`*` glob
///   (`test_delete_node_*prefix*`) a comment uses to point at a group
///   of tests, and equally the shape a doc comment leaves behind when
///   rustfmt breaks a long name across two lines. It resolves when
///   any declared name starts with it.
/// - **one test** otherwise, which has to be declared verbatim.
///
/// Declarations are harvested by text rather than by `syn` on
/// purpose: the throttled-interaction trait tests are emitted by
/// `macro_rules!`, and a parser sees that body as opaque tokens while
/// the `fn test_…` inside it is right there in the text. Over-reading
/// a declaration weakens the check by one name; under-reading it
/// fails a run over a test that exists, which is the worse of the two
/// and the one a parser would produce here.
///
/// Cost: one read of every `.rs` and `.md` file in the workspace; no
/// parse.
pub(crate) fn scan_test_citations() -> CitationScan {
    let sources = workspace_rust_sources();
    let declared = declared_test_names(&sources);
    let mut scan = CitationScan {
        unresolved: Vec::new(),
        resolved: BTreeMap::new(),
    };
    for file in &sources {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        for (number, line) in text.lines().enumerate() {
            let Some(at) = line.find("//") else { continue };
            check_citations(&line[at..], file, number + 1, &declared, &mut scan);
        }
    }
    for file in workspace_markdown_docs() {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        for (number, line) in text.lines().enumerate() {
            check_citations(line, &file, number + 1, &declared, &mut scan);
        }
    }
    scan
}

fn check_citations(
    text: &str,
    file: &Path,
    line: usize,
    declared: &BTreeSet<String>,
    scan: &mut CitationScan,
) {
    for name in test_names_in(text) {
        let where_written = relative_to_repo(file);
        if citation_resolves(&name, declared) {
            *scan.resolved.entry(where_written).or_default() += 1;
        } else {
            scan.unresolved.push(format!("{where_written}:{line}: {name}"));
        }
    }
}

/// Whether `name` points at something the sources declare.
///
/// A name ending in `_` is a family and resolves against any
/// declaration it prefixes; anything else has to be declared
/// verbatim. Resolving *every* citation by prefix would look like a
/// kindness and is not one: a citation of `test_<short>` would then
/// be satisfied by a declaration of `test_<short>_<and_more>`, so the
/// commonest rename there is — growing a test's name — would leave
/// the prose pointing at a function that no longer exists and nothing
/// would say so.
fn citation_resolves(name: &str, declared: &BTreeSet<String>) -> bool {
    if name.ends_with('_') {
        declared.iter().any(|known| known.starts_with(name))
    } else {
        declared.contains(name)
    }
}

/// Every `test_…` identifier in `text` that is not part of a longer
/// word. A trailing `*` is dropped by the character class and leaves
/// the `_` before it, which is what makes a glob read as a family.
fn test_names_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = text[from..].find("test_") {
        let start = from + at;
        let mut end = start + "test_".len();
        while bytes
            .get(end)
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
        {
            end += 1;
        }
        from = end.max(start + 1);
        let preceded_by_word = start > 0 && is_word_byte(bytes[start - 1]);
        if !preceded_by_word && end > start + "test_".len() {
            out.push(text[start..end].to_string());
        }
    }
    out
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Every identifier beginning with `test` that the sources declare —
/// functions, modules, types, constants — plus the stem of every file
/// and directory named that way, since `mod test_utils;` names one.
fn declared_test_names(sources: &[PathBuf]) -> BTreeSet<String> {
    const DECLARING: &[&str] = &[
        "fn",
        "mod",
        "struct",
        "enum",
        "const",
        "static",
        "trait",
        "type",
        "union",
        "macro_rules",
    ];
    let mut out = BTreeSet::new();
    for file in sources {
        for component in relative_to_repo(file).split(['/', '.']) {
            if component.starts_with("test") {
                out.insert(component.to_string());
            }
        }
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        for line in text.lines() {
            let mut previous = "";
            for word in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                if !word.is_empty() && DECLARING.contains(&previous) && word.starts_with("test") {
                    out.insert(word.to_string());
                }
                if !word.is_empty() {
                    previous = word;
                }
            }
        }
    }
    out
}

/// `path` written from the repository root, for a message a reader
/// can paste into an editor.
pub(crate) fn relative_to_repo(path: &Path) -> String {
    path.strip_prefix(repo_path(""))
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// Every shipped write to a `MindNode`'s position *component*, as
/// `(repo-relative file, 1-based line, the line's text)`.
///
/// A component write is one that computes a coordinate —
/// `node.position.x = …`, `node.position.y += …`. Those are the writes
/// that can leave [`MAX_CANVAS_COORD`](crate::mindmap::model::validate::MAX_CANVAS_COORD),
/// and every one of them belongs in
/// [`MindNode::set_position_clamped`](crate::mindmap::model::MindNode::set_position_clamped)
/// or its offset sibling.
///
/// Whole-struct assignment (`node.position = other`) is deliberately
/// **not** reported. It copies a `Position` that is already in the
/// domain rather than deriving a new one: the undo writers restore a
/// snapshot the model held, and `set_node_aabb` takes a caller-supplied
/// position that `validate_node_position` has already rejected on. A
/// scan that flagged those would need a whitelist of call sites, which
/// is the drifting twin surface this exists to avoid.
///
/// Reads shipped text only — [`production_source`] for a test module
/// written inside a file, [`test_gated_module_files`] for a file a
/// `#[cfg(test)] mod x;` reaches — so a fixture that pokes a
/// coordinate on purpose is not a finding.
///
/// Cost: one read and one `syn` parse of every `.rs` file in the
/// workspace.
pub(crate) fn node_position_component_writes() -> Vec<(String, usize, String)> {
    let mut out = Vec::new();
    let sources = workspace_rust_sources();
    // `production_source` blanks a `#[cfg(test)]` item *inside* a file.
    // A file that a `#[cfg(test)] mod tests;` names is test code in its
    // entirety and contains no such marker of its own, so it has to be
    // dropped by the declaration that reaches it — which is what
    // `test_gated_module_files` resolves. Without this the scan reports
    // fixtures that poke a coordinate on purpose.
    let test_only = test_gated_module_files(&sources);
    for file in sources {
        if test_only.contains(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        let shipped = production_source(&file, &text);
        for (number, line) in shipped.lines().enumerate() {
            let code = line.split("//").next().unwrap_or(line);
            let Some(at) = code.find(".position.") else { continue };
            let tail = &code[at + ".position.".len()..];
            let Some(rest) = tail.strip_prefix('x').or_else(|| tail.strip_prefix('y')) else {
                continue;
            };
            // `= ` or `+= ` / `-= ` / `*= ` / `/= `, but not `==`.
            let rest = rest.trim_start();
            let assigns =
                rest.starts_with("= ") || ["+=", "-=", "*=", "/="].iter().any(|op| rest.starts_with(op));
            if assigns {
                out.push((relative_to_repo(&file), number + 1, code.trim().to_string()));
            }
        }
    }
    out
}

/// The `Vec` methods that add exactly **one** element. Only these can
/// be guarded by `room_for_another_section`, which promises room for
/// one more section and nothing beyond.
const SECTION_SINGLE_GROWTH_METHODS: &[&str] = &["push", "insert"];

/// The `Vec` methods that add **any number** of elements. A one-more
/// guard says nothing about these, so they are always reported
/// unguarded: a writer that needs one has to bound its count against
/// `MAX_SECTIONS_PER_NODE` itself and be argued for by name.
const SECTION_MULTI_GROWTH_METHODS: &[&str] = &[
    "extend",
    "extend_from_slice",
    "extend_from_within",
    "append",
    "splice",
    "resize",
    "resize_with",
];

/// Methods whose closure argument runs once per item, so a growth
/// inside it is a growth in a loop. Not exhaustive — a closure passed
/// directly to anything else is assumed to run once, which is how both
/// shipped writers use one (an undo envelope runs its closure once).
/// `map` / `any` / `all` also run at most once on an `Option`; listing
/// them reports that case too, the conservative direction.
const ITERATING_CLOSURE_METHODS: &[&str] = &[
    "for_each",
    "try_for_each",
    "map",
    "map_while",
    "filter",
    "filter_map",
    "flat_map",
    "find",
    "find_map",
    "position",
    "take_while",
    "skip_while",
    "partition",
    "fold",
    "try_fold",
    "scan",
    "inspect",
    "any",
    "all",
    "retain",
    "retain_mut",
    "sort_by",
    "sort_by_key",
    "sort_unstable_by",
    "sort_unstable_by_key",
    "dedup_by",
    "dedup_by_key",
];

/// How a [`SectionGrowth`] writes the vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrowthKind {
    /// `x.sections.push(…)` / `insert(…)`, however rustfmt wraps the
    /// chain: one element, so guardable (in a loop, per iteration).
    Single,
    /// `extend` and the rest of [`SECTION_MULTI_GROWTH_METHODS`]: any
    /// number of elements, never guarded by a one-more check.
    Multi,
    /// `Vec::push(&mut x.sections, …)` — the same call spelled as a
    /// path; one element for `push` / `insert`, else counted as
    /// [`GrowthKind::Multi`].
    Ufcs,
    /// A wholesale replacement: `x.sections = …`,
    /// `x.sections.clone_from(…)`, or `std::mem::{replace, swap, take}`
    /// with `&mut x.sections`. Not a growth by one, and nothing about
    /// the syntax bounds the new length, so it is reported for the
    /// caller to account for by name.
    Assign,
}

/// One site in a Rust source that grows or replaces a `sections`
/// vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SectionGrowth {
    /// Repo-relative file (empty for text scanned with
    /// [`section_growths_in`] directly).
    pub file: String,
    /// 1-based line the growth starts on.
    pub line: usize,
    /// The enclosing fn's name.
    pub function: String,
    pub kind: GrowthKind,
    /// Whether this site is covered by a `room_for_another_section`
    /// guard — see [`section_growths_in`] for exactly what counts.
    pub guarded: bool,
}

/// Every site in `source` that grows or replaces a `sections` vector,
/// with its enclosing fn and whether it is **guarded**.
///
/// Read as syntax (`syn`), not as lines, so the rustfmt-wrapped
/// `node.sections\n    .push(` is the same site as the one-line
/// spelling. A site is any of: a growing method on a `.sections` field
/// receiver ([`SECTION_SINGLE_GROWTH_METHODS`],
/// [`SECTION_MULTI_GROWTH_METHODS`]); the same as a path call whose
/// first argument is `&mut <expr>.sections`; or a replacement
/// ([`GrowthKind::Assign`]) — wherever it sits in the fn body, a
/// closure included (both shipped writers grow inside an undo
/// envelope's closure).
///
/// **Guarded** is deliberately narrow, so a mention cannot pass for a
/// check. A site is guarded only when all of these hold:
///
/// - it adds **one** element: `push` / `insert`, as a method or a path
///   call — `room_for_another_section` promises room for exactly one,
///   so a multi-element method is never guarded by it;
/// - an earlier **top-level** statement of the body that encloses it
///   propagates the guard's refusal. For straight-line code that body
///   is the fn's. For a site in a `for` / `while` / `loop` body it is
///   that loop body — a guard **per iteration** — because a push
///   repeated N times behind one check before the loop is N growths;
///   a site in a loop nested deeper needs a guard in its own body;
/// - it is not inside an **opaque closure**. A closure written directly
///   as a **method** call's argument runs where the call does — the
///   undo-envelope shape both shipped writers grow in — unless the
///   method is one of [`ITERATING_CLOSURE_METHODS`], which run it per
///   item. Any other closure is opaque: one bound with `let`, returned
///   or stored, and every closure argument of a *path* call
///   (`iter::from_fn`, `repeat_with`, `Box::new`, `thread::spawn`, …),
///   which may store it or call it any number of times. Where and how
///   often an opaque closure runs is data flow, which a syntax scan
///   cannot follow, so a growth in it is never guarded rather than
///   guessed at;
/// - the guard has **not been spent**. It promises room for one
///   section against the length when it ran, so it covers the first
///   guardable site after it in its body and no other: a second `push`
///   after one guard — in the same statement, the same envelope
///   closure, a second envelope, or the same iteration — needs a guard
///   of its own. And *any* site recorded after the guard spends it,
///   covered or not: a `Multi` or `Assign` site, one in an opaque
///   closure, or one in a nested loop (each behind its own
///   per-iteration check, which says nothing about the length after the
///   loop) leaves the guard stale for whatever follows.
///
/// The propagating shapes are four: `room_for_another_section(…)`
/// followed only by `map_err(…)` / `ok()` and then `?` (as a statement
/// or a `let` initializer); `let Ok(..) = room_for_another_section(…)
/// else { … }` (the compiler makes the `else` diverge);
/// `if room_for_another_section(…)….is_err() { … }` or
/// `if let Err(..) = room_for_another_section(…) { … }` whose block
/// **ends** in a diverging statement; or a
/// `match room_for_another_section(…)` with an unguarded `Err(..)` arm
/// that ends in one. Diverging means `return`, `break`, `continue`, a
/// `panic!`-family macro or `Err(…)?` — a block that merely *contains*
/// a `return` (`if strict { return … }`) can fall through, and does not
/// count. The call counts as a path (`validate::room_for_another_section(n)`)
/// or a method (`n.room_for_another_section()`). A `?` inside any
/// closure ends the closure, not the fn, so a guard there never counts.
///
/// Not a guard, so reported unguarded: a discarded result (`let _ = …`),
/// a call inside a closure or on a branch, a call after the growth, a
/// refusal swallowed (`or_else(…)?`), inverted (`is_ok()`) or answered
/// only conditionally, or a check in a helper the fn calls.
///
/// Three approximations, stated so nobody reads in more: the guard is
/// **not tied to the growth's receiver** — `room_for_another_section(a)?`
/// covers a later `b.sections.push(…)` — and the receiver's **type is
/// not resolved**, so a `.sections` field on an unrelated struct is a
/// site too (the conservative direction: it is reported, not missed).
/// And a third, in the same direction: guards are spent in source
/// order, not per path, so `room(n)?; if a { push } else { push }`
/// reports the second push although only one of the two runs.
///
/// What this cannot see: a growth through a binding
/// (`let s = &mut node.sections; s.push(…)`), a raw pointer, or code
/// inside a macro invocation's tokens — nor a growth hidden in a call,
/// so `room(n)?; doc.add_section(…)?; n.sections.push(…)` counts the
/// push as covered although the helper made the guard stale.
///
/// Pure over its input so the classification is tested against
/// planted text, not only against whatever the workspace holds.
pub(crate) fn section_growths_in(source: &str) -> Vec<SectionGrowth> {
    use syn::visit::Visit;

    struct Fns {
        out: Vec<SectionGrowth>,
    }
    impl<'ast> Visit<'ast> for Fns {
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            scan_fn_body(&f.sig.ident.to_string(), &f.block, &mut self.out);
            syn::visit::visit_item_fn(self, f);
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            scan_fn_body(&f.sig.ident.to_string(), &f.block, &mut self.out);
            syn::visit::visit_impl_item_fn(self, f);
        }
        fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
            if let Some(block) = &f.default {
                scan_fn_body(&f.sig.ident.to_string(), block, &mut self.out);
            }
            syn::visit::visit_trait_item_fn(self, f);
        }
    }

    let file =
        syn::parse_file(source).unwrap_or_else(|e| panic!("section growth scan: source must parse: {e}"));
    let mut fns = Fns { out: Vec::new() };
    fns.visit_file(&file);
    fns.out.sort_by_key(|g| g.line);
    fns.out
}

/// Scan one fn body's top-level statements in order: collect the
/// growths in each, then let the statement turn the guard on.
fn scan_fn_body(function: &str, body: &syn::Block, out: &mut Vec<SectionGrowth>) {
    let mut guard_seen = false;
    for stmt in &body.stmts {
        let mut finder = GrowthFinder {
            found: Vec::new(),
            loop_depth: 0,
            opaque_closure_depth: 0,
        };
        syn::visit::Visit::visit_stmt(&mut finder, stmt);
        for site in finder.found {
            // One guard is room for one section, checked against the
            // length *then*. Only a guardable site at fn level is covered
            // by it, but **every** site recorded after it — any kind, any
            // loop depth, inside an opaque closure or not — spends it: a
            // growth the guard did not count leaves it stale for the next.
            let covered_here = site.loop_depth == 0 && site.guardable() && guard_seen;
            guard_seen = false;
            let guarded = if site.loop_depth == 0 {
                covered_here
            } else {
                site.guardable() && site.iteration_guarded
            };
            out.push(SectionGrowth {
                file: String::new(),
                line: site.line,
                function: function.to_string(),
                kind: site.kind,
                guarded,
            });
        }
        if propagates_section_guard(stmt) {
            guard_seen = true;
        }
    }
}

/// Is `expr` (after parentheses and references) a field access naming
/// `sections`?
fn is_sections_field(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Field(field) => matches!(&field.member, syn::Member::Named(name) if name == "sections"),
        syn::Expr::Paren(inner) => is_sections_field(&inner.expr),
        syn::Expr::Reference(inner) => is_sections_field(&inner.expr),
        _ => false,
    }
}

/// Is `expr` `&mut <expr>.sections`?
fn is_mut_ref_to_sections(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Reference(r) if r.mutability.is_some() && is_sections_field(&r.expr))
}

/// One site [`GrowthFinder`] found, before the fn-level guard is
/// applied.
struct FoundSite {
    line: usize,
    kind: GrowthKind,
    /// How many loops enclose the site within its statement; 0 is
    /// straight-line code, which only a fn-level guard can cover.
    loop_depth: usize,
    /// Whether an earlier top-level statement of the innermost
    /// enclosing loop body propagates the guard — a per-iteration
    /// guard. Meaningful only when `loop_depth > 0`.
    iteration_guarded: bool,
    /// Whether the site is inside a closure that is **not** passed
    /// directly as an argument — one bound with `let`, returned, or
    /// stored — whose calls, and so how often it grows the vector,
    /// this scan cannot follow. Never guarded.
    in_opaque_closure: bool,
}

impl FoundSite {
    /// Whether a guard could cover this site at all: one element added,
    /// outside an opaque closure. A guard is spent only on such a site.
    fn guardable(&self) -> bool {
        matches!(self.kind, GrowthKind::Single | GrowthKind::Ufcs) && !self.in_opaque_closure
    }
}

/// Collects the sites inside one statement, each with its loop depth
/// and whether its innermost loop body guards every iteration. Nested
/// fn items are skipped — the outer visitor scans them as fns of their
/// own.
struct GrowthFinder {
    found: Vec<FoundSite>,
    loop_depth: usize,
    /// How many opaque closures (see [`FoundSite::in_opaque_closure`])
    /// enclose the current position.
    opaque_closure_depth: usize,
}

impl GrowthFinder {
    fn record(&mut self, line: usize, kind: GrowthKind) {
        self.found.push(FoundSite {
            line,
            kind,
            loop_depth: self.loop_depth,
            iteration_guarded: false,
            in_opaque_closure: self.opaque_closure_depth > 0,
        });
    }

    /// Visit one argument of a (non-iterating) **method** call. A
    /// closure written directly as the argument is walked as code that
    /// runs where the call runs — the undo-envelope shape both shipped
    /// writers grow in. Any other argument goes through the ordinary
    /// visitor, where a closure nested deeper is opaque.
    fn visit_call_arg(&mut self, arg: &syn::Expr) {
        match arg {
            syn::Expr::Closure(closure) => syn::visit::visit_expr_closure(self, closure),
            _ => syn::visit::Visit::visit_expr(self, arg),
        }
    }

    /// Visit a `for` / `while` / `loop` body one level deeper, the way
    /// [`scan_fn_body`] walks a fn body: once a top-level statement of
    /// the body propagates the guard, the sites after it **at this
    /// depth** are guarded per iteration. A site in a loop nested
    /// deeper still needs a guard in its own body.
    fn visit_loop_body(&mut self, body: &syn::Block) {
        self.loop_depth += 1;
        let mut guard_seen = false;
        for stmt in &body.stmts {
            let start = self.found.len();
            syn::visit::Visit::visit_stmt(self, stmt);
            for site in &mut self.found[start..] {
                // As at fn level: the guard covers the first guardable site
                // at this depth after it, and any site at this depth or
                // deeper spends it, covered or not.
                if site.loop_depth < self.loop_depth {
                    continue;
                }
                if guard_seen && site.loop_depth == self.loop_depth && site.guardable() {
                    site.iteration_guarded = true;
                }
                guard_seen = false;
            }
            if propagates_section_guard(stmt) {
                guard_seen = true;
            }
        }
        self.loop_depth -= 1;
    }
}

impl<'ast> syn::visit::Visit<'ast> for GrowthFinder {
    fn visit_item_fn(&mut self, _: &'ast syn::ItemFn) {}

    /// Reached for every closure except one written directly as the
    /// argument of a non-iterating method call (those go through
    /// [`GrowthFinder::visit_call_arg`]).
    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        self.opaque_closure_depth += 1;
        syn::visit::visit_expr_closure(self, closure);
        self.opaque_closure_depth -= 1;
    }

    fn visit_expr_for_loop(&mut self, e: &'ast syn::ExprForLoop) {
        // The iterated expression runs once, before the loop.
        self.visit_expr(&e.expr);
        self.visit_loop_body(&e.body);
    }

    fn visit_expr_while(&mut self, e: &'ast syn::ExprWhile) {
        // The condition runs every iteration, ahead of any guard in
        // the body, so nothing in it is guarded per iteration.
        self.loop_depth += 1;
        self.visit_expr(&e.cond);
        self.loop_depth -= 1;
        self.visit_loop_body(&e.body);
    }

    fn visit_expr_loop(&mut self, e: &'ast syn::ExprLoop) {
        self.visit_loop_body(&e.body);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if is_sections_field(&call.receiver) {
            let line = call.span().start().line;
            if SECTION_SINGLE_GROWTH_METHODS.contains(&method.as_str()) {
                self.record(line, GrowthKind::Single);
            } else if SECTION_MULTI_GROWTH_METHODS.contains(&method.as_str()) {
                self.record(line, GrowthKind::Multi);
            } else if method == "clone_from" {
                self.record(line, GrowthKind::Assign);
            }
        }
        if ITERATING_CLOSURE_METHODS.contains(&method.as_str()) {
            // The receiver runs once; the closure arguments run per item.
            self.visit_expr(&call.receiver);
            self.loop_depth += 1;
            for arg in &call.args {
                self.visit_expr(arg);
            }
            self.loop_depth -= 1;
        } else {
            self.visit_expr(&call.receiver);
            for arg in &call.args {
                self.visit_call_arg(arg);
            }
        }
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            let last = path
                .path
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            let line = call.span().start().line;
            let first_is_sections = call.args.first().is_some_and(is_mut_ref_to_sections);
            if first_is_sections && SECTION_SINGLE_GROWTH_METHODS.contains(&last.as_str()) {
                self.record(line, GrowthKind::Ufcs);
            } else if first_is_sections && SECTION_MULTI_GROWTH_METHODS.contains(&last.as_str()) {
                self.record(line, GrowthKind::Multi);
            } else if ["replace", "swap", "take"].contains(&last.as_str())
                && call.args.iter().any(is_mut_ref_to_sections)
            {
                self.record(line, GrowthKind::Assign);
            }
        }
        self.visit_expr(&call.func);
        // A closure passed to a path call is opaque: `iter::from_fn`,
        // `repeat_with`, `Box::new`, `thread::spawn` and the like store it
        // or call it any number of times, and a path names no receiver
        // the way the undo envelope's method call does.
        for arg in &call.args {
            self.visit_expr(arg);
        }
    }

    fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
        if is_sections_field(&assign.left) {
            self.record(assign.span().start().line, GrowthKind::Assign);
        }
        syn::visit::visit_expr_assign(self, assign);
    }
}

/// Whether `expr` *is* a `room_for_another_section` call — as a path
/// (`validate::room_for_another_section(n)`) or a method
/// (`n.room_for_another_section()`) — optionally followed only by
/// `map_err(…)` and/or `ok()`, which keep the refusal (as `Err` or
/// `None`) rather than swallow it.
fn is_section_guard_call(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Call(call) => matches!(
            &*call.func,
            syn::Expr::Path(path) if path.path.segments.last().is_some_and(|s| s.ident == "room_for_another_section")
        ),
        syn::Expr::MethodCall(call) => {
            call.method == "room_for_another_section"
                || (["map_err", "ok"].contains(&call.method.to_string().as_str())
                    && is_section_guard_call(&call.receiver))
        }
        syn::Expr::Paren(inner) => is_section_guard_call(&inner.expr),
        _ => false,
    }
}

/// The macros that never return.
const DIVERGING_MACROS: &[&str] = &["panic", "unreachable", "todo", "unimplemented"];

fn is_diverging_macro(mac: &syn::Macro) -> bool {
    mac.path
        .segments
        .last()
        .is_some_and(|s| DIVERGING_MACROS.contains(&s.ident.to_string().as_str()))
}

/// Whether `expr` **always** leaves the code that follows it: a
/// `return`, `break` or `continue`, a `panic!`-family macro, an
/// `Err(…)?`, or a block whose last statement is one of those. Merely
/// *containing* a `return` is not enough — `{ if strict { return …; } }`
/// falls through when `strict` is false, and the growth after it runs.
fn diverges(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Return(_) | syn::Expr::Break(_) | syn::Expr::Continue(_) => true,
        syn::Expr::Macro(m) => is_diverging_macro(&m.mac),
        syn::Expr::Try(t) => matches!(
            &*t.expr,
            syn::Expr::Call(call) if matches!(
                &*call.func,
                syn::Expr::Path(path) if path.path.segments.last().is_some_and(|s| s.ident == "Err")
            )
        ),
        syn::Expr::Block(b) => block_diverges(&b.block),
        syn::Expr::Paren(inner) => diverges(&inner.expr),
        _ => false,
    }
}

/// Whether `block` ends in a statement that [`diverges`].
fn block_diverges(block: &syn::Block) -> bool {
    match block.stmts.last() {
        Some(syn::Stmt::Expr(expr, _)) => diverges(expr),
        Some(syn::Stmt::Macro(m)) => is_diverging_macro(&m.mac),
        _ => false,
    }
}

/// Whether `pat` is `Err(..)` / `Ok(..)` (by the variant's last path
/// segment).
fn is_variant_pattern(pat: &syn::Pat, variant: &str) -> bool {
    match pat {
        syn::Pat::TupleStruct(p) => p.path.segments.last().is_some_and(|s| s.ident == variant),
        _ => false,
    }
}

/// Does this top-level statement propagate a `room_for_another_section`
/// refusal out of the fn? See [`section_growths_in`] for the four
/// shapes; anything else is not a guard.
fn propagates_section_guard(stmt: &syn::Stmt) -> bool {
    let try_guard = |expr: &syn::Expr| matches!(expr, syn::Expr::Try(t) if is_section_guard_call(&t.expr));
    match stmt {
        syn::Stmt::Expr(expr, _) => match expr {
            syn::Expr::Try(_) => try_guard(expr),
            syn::Expr::If(i) => {
                let cond_is_refusal = match &*i.cond {
                    // `if room_for_another_section(…).is_err()`
                    syn::Expr::MethodCall(call) => {
                        call.method == "is_err" && is_section_guard_call(&call.receiver)
                    }
                    // `if let Err(..) = room_for_another_section(…)`
                    syn::Expr::Let(l) => is_variant_pattern(&l.pat, "Err") && is_section_guard_call(&l.expr),
                    _ => false,
                };
                cond_is_refusal && block_diverges(&i.then_branch)
            }
            syn::Expr::Match(m) => {
                is_section_guard_call(&m.expr)
                    && m.arms
                        .iter()
                        // An `Err(..) if …` arm parses as `Pat::Guard`, not a
                        // tuple-struct pattern, so it is not counted: it lets the
                        // other refusals fall through to a later arm.
                        .any(|arm| is_variant_pattern(&arm.pat, "Err") && diverges(&arm.body))
            }
            _ => false,
        },
        syn::Stmt::Local(local) => match &local.init {
            Some(init) => {
                try_guard(&init.expr)
                    || (is_variant_pattern(&local.pat, "Ok")
                        && is_section_guard_call(&init.expr)
                        // The compiler requires a `let … else` block to diverge.
                        && init.diverge.is_some())
            }
            None => false,
        },
        _ => false,
    }
}

/// [`section_growths_in`] over every shipped Rust source in the
/// workspace: `production_source` blanks in-file test items, and the
/// out-of-line test modules `test_gated_module_files` resolves are
/// skipped whole, so a fixture that pads a node by direct push is not
/// a finding.
pub(crate) fn section_growth_sites() -> Vec<SectionGrowth> {
    let mut out = Vec::new();
    let sources = workspace_rust_sources();
    let test_only = test_gated_module_files(&sources);
    for file in sources {
        if test_only.contains(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        let rel = relative_to_repo(&file);
        for mut site in section_growths_in(&production_source(&file, &text)) {
            site.file = rel.clone();
            out.push(site);
        }
    }
    out
}

/// Every checked indextree insert in the bodies of the engine's
/// `siblings::insert_sorted` and `siblings::insert_value_at` — the
/// path every new tree node and mutator-tree node goes through, and
/// the insert it places a new node with — as `(repo-relative file, 1-based
/// line, the line's text)`, plus the count of the value-inserting
/// calls the body makes, so a caller can tell an empty finding from a
/// scan that read nothing.
///
/// `indextree`'s `append`, `prepend`, `insert_after` and
/// `insert_before` are wrappers over their `checked_*` forms, whose
/// cycle check is `ancestors(arena).any(..)` — an O(depth) walk, on
/// every insert. The scene builder inserts once per node and once per
/// section, and a `.mindmap.json` may declare a linear `parent_id`
/// chain, which is a legal acyclic tree the loader accepts. Depth then
/// equals the node count and the build is O(N²).
///
/// The `*_value` forms are indextree's documented O(1) fast paths.
/// `insert_sorted` places a value it is handed, so the node is new and
/// detached by construction and the ancestor walk could never fire —
/// the check is pure cost.
pub(crate) fn sorted_insert_checked_calls() -> (Vec<(String, usize, String)>, usize) {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/engine/siblings.rs");
    let rel = relative_to_repo(&file);
    let text =
        std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
    let source = production_source(&file, &text);
    let mut found = Vec::new();
    let mut value_inserts = 0;
    let mut in_body = false;
    for (number, line) in source.lines().enumerate() {
        if line.starts_with("pub(crate) fn insert_sorted")
            || line.starts_with("pub(crate) fn insert_value_at")
        {
            in_body = true;
        } else if in_body && line == "}" {
            in_body = false;
            continue;
        }
        if !in_body {
            continue;
        }
        let code = line.split("//").next().unwrap_or(line);
        for call in [
            "append_value(",
            "prepend_value(",
            "insert_after_value(",
            "insert_before_value(",
        ] {
            value_inserts += code.matches(call).count();
        }
        let checked = [
            ".append(",
            ".prepend(",
            ".insert_after(",
            ".insert_before(",
            "checked_",
        ];
        if checked.iter().any(|call| code.contains(call)) {
            found.push((rel.clone(), number + 1, code.trim().to_string()));
        }
    }
    (found, value_inserts)
}

/// The climbs to the root and the inserts of existing nodes in the
/// engine's shipped code (`lib/baumhard/src/engine/`, its `tests`
/// tree excepted) that are **not** in one of the functions allowed to
/// make them, as `(repo-relative file, 1-based line, the line's text)`,
/// plus, for each allowed function, how many such calls it makes — so
/// a caller can tell an empty finding from a scan that read nothing,
/// and an allowance nothing uses any more from one still needed.
///
/// Two kinds of call are found:
///
/// - **indextree's own climbs**, by line: `ancestors`, and its insert
///   of an existing node (`checked_*`, and `append` / `prepend` /
///   `insert_after` / `insert_before` / `reparent`, which wrap them),
///   which climbs to check for a cycle.
/// - **a hand-written climb**, by syntax: a parent read — a `parent`
///   method, or a function or method whose name starts `parent_` —
///   in one of the three spellings of walking up a chain of parents
///   whatever the cursor is called: inside a `loop` or a `while` (its
///   condition included); inside a closure handed to an iterator that
///   feeds a value back into itself (`successors`, `from_fn`, `scan`);
///   or inside a `for` whose body assigns a variable the read itself
///   names (`cur = p` beside `arena[cur].parent()`). A parent read in
///   any other `for` is not one: that `for` walks what it is given,
///   once per item.
///
/// It is a heuristic over spellings, not a proof. A climb by
/// recursion is not read here — the deep tests' 100,000-level chains
/// overflow the stack on one — and neither is one hidden behind a
/// helper that reads one parent per call and is called in a loop.
///
/// The engine keeps each in a few places: `tree::climb` (counted in
/// test builds), and `siblings`' `attach_after` / `attach_first`
/// behind `checked_attach` (counted) for indextree's; and for
/// hand-written ones the memoized climbs — `Membership::contains`
/// (counted), the journal's `Presence::top` (counted) and its
/// `resolve` — and the walker's `state_above`, the one-off climb an
/// anchored walk makes to its anchor's parent. So a climb planted once
/// per element anywhere else shows here, and one planted through those
/// functions shows in their step counts (§20 rule 11, V1-A5 #282).
pub(crate) fn engine_depth_cost_calls() -> EngineDepthCosts {
    const ALLOWED: &[(&str, &str)] = &[
        ("tree.rs", "climb"),
        ("siblings.rs", "attach_after"),
        ("siblings.rs", "attach_first"),
    ];
    const ALLOWED_PARENT_LOOPS: &[(&str, &str)] = &[
        ("membership.rs", "contains"),
        ("journal.rs", "top"),
        ("journal.rs", "resolve"),
        ("walker.rs", "state_above"),
    ];
    const CALLS: &[&str] = &[
        ".ancestors(",
        ".checked_insert_after(",
        ".checked_insert_before(",
        ".checked_prepend(",
        ".checked_append(",
        ".checked_reparent(",
        ".append(",
        ".prepend(",
        ".insert_after(",
        ".insert_before(",
        ".reparent(",
    ];
    let root = repo_path("lib/baumhard/src/engine");
    let mut files = Vec::new();
    collect_recursively(&root, ".rs", &mut files);
    files.sort();
    let mut found = Vec::new();
    let mut allowed: BTreeMap<(String, String), usize> = ALLOWED
        .iter()
        .chain(ALLOWED_PARENT_LOOPS)
        .map(|&(file, function)| ((file.to_string(), function.to_string()), 0))
        .collect();
    for file in files {
        if file.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        let name = file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        let source = production_source(&file, &text);
        let lines: Vec<&str> = source.lines().collect();
        let mut current_fn = String::new();
        for (number, line) in lines.iter().enumerate() {
            let code = line.split("//").next().unwrap_or(line);
            if let Some(at) = code.find("fn ") {
                let after = &code[at + 3..];
                let ident: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !ident.is_empty() {
                    current_fn = ident;
                }
            }
            // A Vec's `append` is not indextree's: only a call handed
            // an arena is.
            let calls = CALLS
                .iter()
                .any(|call| code.contains(call) && (*call != ".append(" || code.contains("arena")));
            if !calls {
                continue;
            }
            if ALLOWED.contains(&(name.as_str(), current_fn.as_str())) {
                *allowed.entry((name.clone(), current_fn.clone())).or_default() += 1;
            } else {
                found.push((relative_to_repo(&file), number + 1, code.trim().to_string()));
            }
        }
        for (function, line) in parent_reads_in_loops(&file, &source) {
            if ALLOWED_PARENT_LOOPS.contains(&(name.as_str(), function.as_str())) {
                *allowed.entry((name.clone(), function)).or_default() += 1;
            } else {
                let code = lines.get(line - 1).map_or("", |l| l.trim());
                found.push((relative_to_repo(&file), line, code.to_string()));
            }
        }
    }
    found.sort();
    found.dedup();
    EngineDepthCosts { found, allowed }
}

/// What [`engine_depth_cost_calls`] found.
pub(crate) struct EngineDepthCosts {
    /// Each call outside the allowed functions: `(repo-relative file,
    /// 1-based line, the line's text)`, one per line.
    pub(crate) found: Vec<(String, usize, String)>,
    /// Each allowed `(file name, function)`, with how many calls the
    /// scan read in it; zero means the allowance is unused.
    pub(crate) allowed: BTreeMap<(String, String), usize>,
}

/// Every parent read of `source` — a `parent` method, or a function or
/// method named `parent_*` — that climbs by [`engine_depth_cost_calls`]'
/// three spellings: inside a `loop` or a `while`; inside a closure
/// handed to `successors`, `from_fn` or `scan`; or inside a `for` whose
/// body assigns a plain variable the read names (in its receiver or its
/// arguments). As `(the enclosing function's name, 1-based line)`. A
/// closure counts where it is written. Panics when `source` does not
/// parse, naming `file`.
fn parent_reads_in_loops(file: &Path, source: &str) -> Vec<(String, usize)> {
    use std::collections::BTreeSet;
    use syn::visit::{self, Visit};

    /// The iterator constructors and adapters whose closure is handed
    /// its own last result: a parent read there is a climb.
    const FEEDBACK: &[&str] = &["successors", "from_fn", "scan"];

    struct Finder {
        function: String,
        loops: usize,
        /// For each enclosing `for`, the plain variables its body
        /// assigns.
        cursors: Vec<BTreeSet<String>>,
        found: Vec<(String, usize)>,
    }

    /// The plain variables `block` assigns anywhere inside it.
    fn assigned(block: &syn::Block) -> BTreeSet<String> {
        struct Assigned(BTreeSet<String>);
        impl<'ast> Visit<'ast> for Assigned {
            fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
                if let syn::Expr::Path(path) = &*assign.left {
                    if let Some(ident) = path.path.get_ident() {
                        self.0.insert(ident.to_string());
                    }
                }
                visit::visit_expr_assign(self, assign);
            }
        }
        let mut found = Assigned(BTreeSet::new());
        found.visit_block(block);
        found.0
    }

    /// The plain variables `exprs` name.
    fn named<'a>(exprs: impl IntoIterator<Item = &'a syn::Expr>) -> BTreeSet<String> {
        struct Named(BTreeSet<String>);
        impl<'ast> Visit<'ast> for Named {
            fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
                if let Some(ident) = path.path.get_ident() {
                    self.0.insert(ident.to_string());
                }
                visit::visit_expr_path(self, path);
            }
        }
        let mut found = Named(BTreeSet::new());
        for expr in exprs {
            found.visit_expr(expr);
        }
        found.0
    }

    impl Finder {
        /// A call or method named `name` reading `operands` (receiver
        /// and arguments): recorded when it is a parent read that climbs.
        fn read<'a>(&mut self, name: &syn::Ident, operands: impl IntoIterator<Item = &'a syn::Expr>) {
            let text = name.to_string();
            if text != "parent" && !text.starts_with("parent_") {
                return;
            }
            let climbs = self.loops > 0 || {
                let names = named(operands);
                self.cursors.iter().any(|assigned| !assigned.is_disjoint(&names))
            };
            if climbs {
                self.found.push((self.function.clone(), name.span().start().line));
            }
        }

        fn in_function(&mut self, name: &syn::Ident, body: impl FnOnce(&mut Self)) {
            let function = std::mem::replace(&mut self.function, name.to_string());
            let loops = std::mem::take(&mut self.loops);
            let cursors = std::mem::take(&mut self.cursors);
            body(self);
            self.function = function;
            self.loops = loops;
            self.cursors = cursors;
        }

        /// Visit `args` as a loop body: they hold a closure the callee
        /// feeds its own results.
        fn feedback<'ast>(&mut self, args: impl IntoIterator<Item = &'ast syn::Expr>) {
            self.loops += 1;
            for arg in args {
                self.visit_expr(arg);
            }
            self.loops -= 1;
        }
    }

    impl<'ast> Visit<'ast> for Finder {
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            self.in_function(&item.sig.ident, |me| visit::visit_item_fn(me, item));
        }

        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            self.in_function(&item.sig.ident, |me| visit::visit_impl_item_fn(me, item));
        }

        fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
            self.in_function(&item.sig.ident, |me| visit::visit_trait_item_fn(me, item));
        }

        fn visit_expr_loop(&mut self, expr: &'ast syn::ExprLoop) {
            self.loops += 1;
            visit::visit_expr_loop(self, expr);
            self.loops -= 1;
        }

        fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
            self.loops += 1;
            visit::visit_expr_while(self, expr);
            self.loops -= 1;
        }

        fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
            self.visit_expr(&expr.expr);
            self.cursors.push(assigned(&expr.body));
            self.visit_block(&expr.body);
            self.cursors.pop();
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            self.read(&call.method, std::iter::once(&*call.receiver).chain(&call.args));
            if FEEDBACK.contains(&call.method.to_string().as_str()) {
                self.visit_expr(&call.receiver);
                self.feedback(&call.args);
            } else {
                visit::visit_expr_method_call(self, call);
            }
        }

        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            let mut feeds_back = false;
            if let syn::Expr::Path(path) = &*call.func {
                if let Some(last) = path.path.segments.last() {
                    self.read(&last.ident, &call.args);
                    feeds_back = FEEDBACK.contains(&last.ident.to_string().as_str());
                }
            }
            if feeds_back {
                self.feedback(&call.args);
            } else {
                visit::visit_expr_call(self, call);
            }
        }
    }

    let parsed =
        syn::parse_file(source).unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
    let mut finder = Finder {
        function: String::new(),
        loops: 0,
        cursors: Vec::new(),
        found: Vec::new(),
    };
    finder.visit_file(&parsed);
    finder.found
}

/// Every write to a field named `root` in the workspace's shipped code
/// — `lib/`, `src/` and `crates/`, minus `tests/` directories, benches,
/// test-gated modules and test-gated items — outside
/// `lib/baumhard/src/engine/tree.rs`, as `(repo-relative file, 1-based
/// line, the line's text)`, plus how many `root` fields the scan read
/// at all, reads included, so a caller can tell an empty finding from
/// a scan that read nothing.
///
/// A write is an assignment to `<expr>.root` (plain or compound) or a
/// `&mut <expr>.root` — which is how `std::mem::replace`, `swap` and
/// `take`, or any function that writes through a reference, reach it.
/// `Tree::root` is a public field, kept public for its reads; writing
/// it would move the tree's root without the engine knowing, and the
/// set behind `Tree::contains` and every journal with it would then
/// describe another tree. The scan cannot see types, so it refuses a
/// write to any field of that name: none exists outside the engine
/// today, and one that is not a `Tree`'s can be renamed or allowed by
/// name with its reason.
///
/// Cost: one read and one `syn` parse of every `.rs` file in those
/// roots.
pub(crate) fn root_field_writes() -> (Vec<(String, usize, String)>, usize) {
    use syn::visit::{self, Visit};

    struct Writes {
        lines: Vec<usize>,
        fields: usize,
    }

    fn is_root_field(expr: &syn::Expr) -> Option<usize> {
        match expr {
            syn::Expr::Field(field) => match &field.member {
                syn::Member::Named(name) if name == "root" => Some(name.span().start().line),
                _ => None,
            },
            syn::Expr::Paren(inner) => is_root_field(&inner.expr),
            _ => None,
        }
    }

    impl<'ast> Visit<'ast> for Writes {
        fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
            if matches!(&field.member, syn::Member::Named(name) if name == "root") {
                self.fields += 1;
            }
            visit::visit_expr_field(self, field);
        }

        fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
            if let Some(line) = is_root_field(&assign.left) {
                self.lines.push(line);
            }
            visit::visit_expr_assign(self, assign);
        }

        fn visit_expr_binary(&mut self, binary: &'ast syn::ExprBinary) {
            let compound = matches!(
                binary.op,
                syn::BinOp::AddAssign(_)
                    | syn::BinOp::SubAssign(_)
                    | syn::BinOp::MulAssign(_)
                    | syn::BinOp::DivAssign(_)
                    | syn::BinOp::RemAssign(_)
                    | syn::BinOp::BitXorAssign(_)
                    | syn::BinOp::BitAndAssign(_)
                    | syn::BinOp::BitOrAssign(_)
                    | syn::BinOp::ShlAssign(_)
                    | syn::BinOp::ShrAssign(_)
            );
            if compound {
                if let Some(line) = is_root_field(&binary.left) {
                    self.lines.push(line);
                }
            }
            visit::visit_expr_binary(self, binary);
        }

        fn visit_expr_reference(&mut self, reference: &'ast syn::ExprReference) {
            if reference.mutability.is_some() {
                if let Some(line) = is_root_field(&reference.expr) {
                    self.lines.push(line);
                }
            }
            visit::visit_expr_reference(self, reference);
        }
    }

    let sources = workspace_rust_sources();
    let test_only = test_gated_module_files(&sources);
    let mut found = Vec::new();
    let mut fields = 0;
    for file in sources {
        let rel = relative_to_repo(&file);
        let test_path = rel.split('/').any(|part| part == "tests" || part == "benches");
        if test_path || test_only.contains(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        let source = production_source(&file, &text);
        let parsed =
            syn::parse_file(&source).unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
        let mut writes = Writes {
            lines: Vec::new(),
            fields: 0,
        };
        writes.visit_file(&parsed);
        fields += writes.fields;
        if rel == "lib/baumhard/src/engine/tree.rs" {
            continue;
        }
        let lines: Vec<&str> = source.lines().collect();
        for line in writes.lines {
            let code = lines.get(line - 1).map_or("", |l| l.trim());
            found.push((rel.clone(), line, code.to_string()));
        }
    }
    found.sort();
    found.dedup();
    (found, fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_temp::TempDir;

    /// The five path prefixes [`scan_test_citations`] must be able to
    /// show its work in.
    ///
    /// A scan whose answer is "nothing is wrong" passes just as
    /// loudly when it has stopped looking, and pointing it at a root
    /// that moved would do exactly that — silently, since a missing
    /// citation is not an error. Each prefix here carries citations
    /// today, so a walk that lost one goes red naming the prefix.
    const CITATION_WITNESSES: &[&str] = &[
        "TEST_CONVENTIONS.md",
        "format/",
        "lib/baumhard/src/",
        "src/application/",
        "crates/",
    ];

    fn parse_attrs(source: &str) -> Vec<Attribute> {
        syn::parse::Parser::parse_str(Attribute::parse_outer, source)
            .unwrap_or_else(|e| panic!("the test's own attribute must parse: {e}\n{source}"))
    }

    /// The `cfg` reader has a list-form blind spot if it looks only
    /// for the bare `test` ident, and it was live: this crate gates
    /// its native-only test modules with `#[cfg(all(test,
    /// not(target_arch = "wasm32")))]`, and reading the bare ident
    /// alone classified every one of them as production source.
    #[test]
    fn test_a_test_gate_written_as_all_is_still_a_test_gate() {
        for source in [
            "#[cfg(test)]",
            r#"#[cfg(all(test, not(target_arch = "wasm32")))]"#,
            "#[cfg(all(unix, all(test)))]",
        ] {
            assert!(
                is_test_gated(&parse_attrs(source)),
                "{source} compiles its item only under `cfg(test)`, so the item is not \
                 part of any shipped path and must be excised"
            );
        }
        for source in [
            "#[cfg(not(test))]",
            r#"#[cfg(any(test, feature = "x"))]"#,
            r#"#[cfg(target_arch = "wasm32")]"#,
        ] {
            assert!(
                !is_test_gated(&parse_attrs(source)),
                "{source} does not gate on `test`, and treating it as a test gate would \
                 delete shipped code from every scan built on this"
            );
        }
    }

    /// **The premise the loader's scan used to rest on, stated and
    /// refuted.** It split each file at the first `#[cfg(test)]` and
    /// kept the part before it, on the belief that test modules hang
    /// off the *end* of a file. 140 of the 321 files it collected
    /// carry a gate before the end, and 28 of those go on to declare
    /// shipped code below it — `src/application/document/mod.rs` was
    /// read to line 35 of 757. A `serde_json::to_string_pretty(&map)`
    /// planted below such a gate passed the guard that exists to
    /// forbid it.
    #[test]
    fn test_production_source_keeps_code_written_after_a_test_gate() {
        let source = "#[cfg(test)]\nmod tests;\n\npub fn shipped() {}\n";
        let production = production_source(Path::new("planted.rs"), source);
        assert!(
            production.contains("pub fn shipped"),
            "code after an out-of-line test module is shipped and must survive: {production:?}"
        );
        assert!(
            !production.contains("mod tests"),
            "the gated declaration itself is not shipped: {production:?}"
        );
    }

    /// Line numbers are the excision's other half: a caller reports
    /// `file:line`, so a blanked region has to keep its height. The
    /// truncating split failed exactly here — it did not shorten the
    /// text, it *ended* it, and nothing counted the lines that went
    /// missing with it.
    #[test]
    fn test_production_source_keeps_every_line_at_its_own_number() {
        let source = "pub fn a() {}\n#[cfg(test)]\nmod tests {\n    fn hidden() {}\n}\npub fn b() {}\n";
        let production = production_source(Path::new("planted.rs"), source);
        let lines: Vec<&str> = production.lines().collect();
        assert_eq!(lines.len(), source.lines().count(), "height must not change");
        assert_eq!(lines[0], "pub fn a() {}");
        assert_eq!(lines[5], "pub fn b() {}");
        for blanked in &lines[1..5] {
            assert!(
                blanked.is_empty(),
                "the gated module's lines must be blank: {lines:?}"
            );
        }
    }

    /// **A file is allowed to gate itself, and three in this
    /// workspace do.** `#![cfg(test)]` is an inner attribute on the
    /// file, not on any item in it, so the item walk cannot see it.
    /// For two of the three — `src/application/macros/tests.rs` and
    /// `…/cross_dispatch/selection/tests.rs` — the `mod tests;` that
    /// names the file carries no attribute either, so
    /// [`test_gated_module_files`] does not name them and nothing
    /// else would: every line of both read as shipped code until
    /// this was handled, and the first scan built on this reader
    /// duly reported twenty-six test `unwrap()`s (nineteen and
    /// seven) as production offenders. The third,
    /// `crates/maptool/src/verify/test_helpers.rs`, is declared
    /// `#[cfg(test)] pub(crate) mod test_helpers;` and was already
    /// excluded by the sibling mechanism — it contributed none of
    /// the twenty-six.
    ///
    /// The peer half of the assertion is what keeps it from passing
    /// on a reader that blanks any file carrying an inner attribute.
    #[test]
    fn test_production_source_blanks_a_file_that_gates_itself() {
        let gated = production_source(Path::new("planted.rs"), "#![cfg(test)]\n\npub fn helper() {}\n");
        assert!(
            gated.trim().is_empty(),
            "a file behind `#![cfg(test)]` ships nothing at all: {gated:?}"
        );

        // The peer, differing only in which inner attribute it
        // carries. Without it this test is satisfied by a reader
        // that blanks every file with an inner attribute, or every
        // file at all — and the line number is checked here rather
        // than above because a fully blanked file has no surviving
        // line to check it on.
        let ungated = production_source(
            Path::new("planted.rs"),
            "#![allow(dead_code)]\n\npub fn helper() {}\n",
        );
        assert_eq!(
            ungated.lines().nth(2),
            Some("pub fn helper() {}"),
            "an inner attribute that is not a test gate must leave the file, and its line \
             numbering, alone: {ungated:?}"
        );
    }

    /// Test code is not only ever a module. A `#[cfg(test)]` helper
    /// inside an `impl` block or a `trait` body is test code by the
    /// same rule, and leaving it in the text would report a test
    /// helper as a shipped path.
    #[test]
    fn test_production_source_reaches_a_gated_member_of_an_impl_or_trait() {
        let source = "impl S {\n    #[cfg(test)]\n    fn helper() {}\n    pub fn real() {}\n}\n\
                      trait T {\n    #[cfg(test)]\n    fn probe() {}\n}\n";
        let production = production_source(Path::new("planted.rs"), source);
        assert!(production.contains("pub fn real"), "{production:?}");
        assert!(!production.contains("fn helper"), "{production:?}");
        assert!(!production.contains("fn probe"), "{production:?}");
    }

    fn plant_module_tree(dir: &TempDir) {
        std::fs::write(
            dir.join("lib.rs"),
            "#[cfg(all(test, not(target_arch = \"wasm32\")))]\nmod hidden;\nmod shown;\n",
        )
        .expect("planted root must be writable");
        std::fs::create_dir_all(dir.join("hidden")).expect("planted directory must be creatable");
        std::fs::write(
            dir.join("hidden").join("mod.rs"),
            "mod deeper;\npub struct Hidden;\n",
        )
        .expect("planted module must be writable");
        std::fs::write(dir.join("hidden").join("deeper.rs"), "pub struct Deeper;\n")
            .expect("planted submodule must be writable");
        std::fs::write(dir.join("shown.rs"), "pub struct Shown;\n").expect("planted peer must be writable");
    }

    /// **The half a filesystem walk cannot see.** `#[cfg(test)] mod
    /// tests { … }` is an attribute on an item the parser holds;
    /// `#[cfg(test)] mod tests;` is an attribute on an item whose body
    /// is another file, and a walk over files never meets the
    /// attribute at all. `util/mod.rs` gates three modules that way
    /// and every type they declare was indexed as a model type.
    ///
    /// Transitivity is the second half: a gated module's own
    /// submodules are test sources because their parent is, and
    /// nothing in *their* text says so.
    #[test]
    fn test_an_out_of_line_test_module_and_its_subtree_are_excluded() {
        let dir = TempDir::new("source-scan-modules");
        plant_module_tree(&dir);
        let excluded = test_gated_module_files(&[dir.join("lib.rs")]);
        assert!(
            excluded.contains(&dir.join("hidden").join("mod.rs")),
            "the gated module's own file must be excluded: {excluded:?}"
        );
        assert!(
            excluded.contains(&dir.join("hidden").join("deeper.rs")),
            "and everything it declares, which says nothing about itself: {excluded:?}"
        );
        assert!(
            !excluded.contains(&dir.join("shown.rs")),
            "an ungated peer is shipped source and must stay: {excluded:?}"
        );
    }

    /// A module this resolver cannot follow stops the run rather than
    /// dropping out of the excluded set — the same posture
    /// [`crate::util::manifests`] takes toward a manifest shape it
    /// cannot read, and for the same reason: a silent skip turns a
    /// guard into a test that cannot fail.
    #[test]
    #[should_panic(expected = "does not follow")]
    fn test_a_path_attribute_on_a_module_stops_the_run() {
        let dir = TempDir::new("source-scan-path-attr");
        std::fs::write(
            dir.join("lib.rs"),
            "#[cfg(test)]\n#[path = \"elsewhere.rs\"]\nmod hidden;\n",
        )
        .expect("planted root must be writable");
        let _ = test_gated_module_files(&[dir.join("lib.rs")]);
    }

    /// See [`test_a_path_attribute_on_a_module_stops_the_run`]: a
    /// `mod` naming a file that is not on disk is the same kind of
    /// gap.
    #[test]
    #[should_panic(expected = "needs exactly one")]
    fn test_a_module_with_no_file_stops_the_run() {
        let dir = TempDir::new("source-scan-missing-module");
        std::fs::write(dir.join("lib.rs"), "#[cfg(test)]\nmod absent;\n")
            .expect("planted root must be writable");
        let _ = test_gated_module_files(&[dir.join("lib.rs")]);
    }

    /// A citation ending in `_` names a family, which is what a
    /// comment writes when it means "these tests" — and, by
    /// coincidence that costs nothing, also what a name broken across
    /// two lines of a doc comment leaves behind.
    #[test]
    fn test_a_citation_is_read_as_a_family_only_when_it_ends_in_an_underscore() {
        assert_eq!(
            test_names_in("see `tests_delete::test_delete_node_*prefix*` for the case"),
            vec!["test_delete_node_".to_string()]
        );
        assert_eq!(
            test_names_in("/// `test_hit_test_direct_hit` is the exemplar"),
            vec!["test_hit_test_direct_hit".to_string()]
        );
        assert!(
            test_names_in("no_test_foo and pretest_bar name nothing").is_empty(),
            "a `test_` inside a longer word is not a citation"
        );
    }

    /// And the family rule has to stay the exception. Resolving every
    /// citation by prefix would look generous and would hide the
    /// commonest rename there is: growing a test's name. Prose naming
    /// the short spelling would then be satisfied by the long one and
    /// point at a function that no longer exists.
    #[test]
    fn test_a_citation_that_only_prefixes_a_declared_name_does_not_resolve() {
        let declared: BTreeSet<String> = ["test_delete_node_by_prefix".to_string()].into_iter().collect();
        assert!(
            !citation_resolves("test_delete_node", &declared),
            "a whole-name citation must name the whole name"
        );
        assert!(
            citation_resolves("test_delete_node_", &declared),
            "written as a family, the same text resolves"
        );
        assert!(
            citation_resolves("test_delete_node_by_prefix", &declared),
            "and the exact name always does"
        );
    }

    /// **Every test named in a comment or a spec is a real one.**
    ///
    /// Two citations had already rotted before this existed, both
    /// found by hand in review and both of the same shape: a test was
    /// renamed and the prose that pointed at it was not.
    /// `format/animation-roadmap.md` named the `Followup` gate by a
    /// spelling that has never existed in this repository — the test
    /// it means has since been renamed twice underneath it. The dead
    /// spelling is not written here on purpose: this scan would
    /// report it, which is the point.
    ///
    /// A stale citation is not a typo. These documents are the map a
    /// reader uses to find the check behind a claim, and a pointer
    /// into nothing reads as "the claim is untested" or, worse, sends
    /// them looking for a guarantee nobody wrote.
    #[test]
    fn test_no_doc_cites_a_test_that_does_not_exist() {
        let scan = scan_test_citations();
        assert!(
            scan.unresolved.is_empty(),
            "{} citation(s) name a `test_*` the sources do not declare. Either the test \
             was renamed and the prose was not, or the prose means a *family* — write \
             that as a trailing `*` (`test_delete_node_*prefix*`) and it reads as \
             one:\n  {}",
            scan.unresolved.len(),
            scan.unresolved.join("\n  ")
        );
    }

    /// The scan above answers "nothing is wrong" just as loudly when
    /// it has stopped looking, so what it *did* read is pinned: each
    /// witness prefix carries citations today, and a root that moved
    /// out from under [`RUST_ROOTS`] / [`MARKDOWN_ROOTS`] goes red
    /// naming itself instead of quietly shrinking the checked set.
    #[test]
    fn test_the_citation_scan_reads_every_place_it_claims_to() {
        let scan = scan_test_citations();
        for witness in CITATION_WITNESSES {
            let found: usize = scan
                .resolved
                .iter()
                .filter(|(file, _)| file.starts_with(witness))
                .map(|(_, count)| count)
                .sum();
            assert!(
                found > 0,
                "the citation scan resolved nothing under `{witness}`, so it is no longer \
                 looking there — a stale citation in it would pass in silence"
            );
        }
    }

    /// The climb spellings `parent_reads_in_loops` recognizes, and the
    /// `for` it does not: one parent read per item of what a `for` is
    /// given climbs nothing.
    #[test]
    fn test_parent_reads_in_loops_reads_the_climb_spellings() {
        let found = |body: &str| {
            let source = format!("fn f() {{ {body} }}");
            parent_reads_in_loops(Path::new("probe.rs"), &source).len()
        };
        assert_eq!(found("while let Some(n) = at { at = arena[n].parent(); }"), 1);
        assert_eq!(
            found("loop { match arena[at].parent() { Some(p) => at = p, None => break } }"),
            1
        );
        assert_eq!(
            found("let _ = std::iter::successors(Some(t), |&n| arena[n].parent()).count();"),
            1
        );
        assert_eq!(
            found("let _ = std::iter::from_fn(|| { at = arena[at].parent()?; Some(at) });"),
            1
        );
        assert_eq!(
            found("let _ = ids.iter().scan(t, |at, _| { *at = arena[*at].parent()?; Some(*at) });"),
            1
        );
        assert_eq!(
            found("for _ in 0..n { match arena[cur].parent() { Some(p) => cur = p, None => break } }"),
            1
        );
        assert_eq!(found("while let Some(n) = at { at = parent_of(n); }"), 1);
        assert_eq!(
            found("for id in ids { if let Some(p) = arena[id].parent() { out.push(p); } }"),
            0
        );
        assert_eq!(
            found("for id in ids { let p = arena[id].parent(); last = p; }"),
            0
        );
        assert_eq!(found("let p = arena[n].parent();"), 0);
        assert_eq!(
            found("let _ = ids.iter().map(|&n| arena[n].parent()).count();"),
            0
        );
    }

    /// **Only `engine/tree.rs` writes a `root` field** in the workspace's
    /// shipped code ([`root_field_writes`]). `Tree::root` stays a public
    /// field for its reads, so this is what keeps a tree's root where
    /// the engine put it — and with it `Tree::contains` and every
    /// journal — until the field can be made private.
    #[test]
    fn test_only_the_engine_tree_writes_a_root_field() {
        let (found, fields) = root_field_writes();
        assert!(
            fields > 0,
            "precondition: the scan reads the workspace's `root` fields, {fields} of them"
        );
        assert!(
            found.is_empty(),
            "a write to a `root` field outside lib/baumhard/src/engine/tree.rs — `Tree::root` \
             is written only by the engine:\n{}",
            found
                .iter()
                .map(|(file, line, code)| format!("  {file}:{line}   {code}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
