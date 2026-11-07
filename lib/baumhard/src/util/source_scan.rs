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
/// - **the arena's own climbs**, by line: `ancestors`, and its insert
///   of an existing node under a new parent (`checked_append`,
///   `checked_prepend`, `checked_insert_after`,
///   `checked_insert_before`), which climbs to check for a cycle.
/// - **a hand-written climb**, by syntax: a parent read — a `parent`
///   method, or a function or method whose name starts `parent_` —
///   in one of the three spellings of walking up a chain of parents
///   whatever the cursor is called: inside a `loop` or a `while` (its
///   condition included); inside a closure handed to an iterator that
///   feeds it its own last result (`successors`, `from_fn`,
///   `repeat_with`, `scan`, the folds, `reduce`); or inside a `for`
///   body or any closure that assigns a variable the read itself
///   names, directly or through a `*` deref (`cur = p` beside
///   `arena[cur].parent()`). A parent read in any other `for` or
///   closure is not one: it reads one parent per item it is given.
///
/// It is a heuristic over spellings, not a proof. A climb by
/// recursion is not read here — the deep tests' 100,000-level chains
/// overflow the stack on one — and neither is one hidden behind a
/// helper that reads one parent per call and is called in a loop.
///
/// The engine keeps each in a few places: `tree::climb` (counted in
/// test builds), and `siblings::attach_at` (counted), the one checked
/// insert, for the arena's; and for
/// hand-written ones the memoized climbs — `Membership::contains`
/// (counted), the journal's `Presence::top` (counted) and its
/// `resolve` — and the walker's `state_above`, the one-off climb an
/// anchored walk makes to its anchor's parent; in the document, the
/// three named in `ALLOWED_PARENT_LOOPS`. The scan reads the engine and
/// the document model built on it, its format modules included. So a
/// climb planted once per element anywhere else shows here, and one
/// planted through those functions shows in their step counts (§20
/// rule 11, V1-A5 #282).
pub(crate) fn engine_depth_cost_calls() -> EngineDepthCosts {
    /// The trees read: the engine, and the document model built on it
    /// (its format modules included), which walk trees of any depth.
    const SCANNED: &[&str] = &["lib/baumhard/src/engine", "lib/baumhard/src/document"];
    const ALLOWED: &[(&str, &str)] = &[("engine/tree.rs", "climb"), ("engine/siblings.rs", "attach_at")];
    const ALLOWED_PARENT_LOOPS: &[(&str, &str)] = &[
        ("engine/membership.rs", "contains"),
        ("engine/journal.rs", "top"),
        ("engine/journal.rs", "resolve"),
        ("engine/walker.rs", "state_above"),
        // The document's three: B3's memoized climb over a selection,
        // which climbs each ancestor once however many nodes are given
        // (and counts it); the inherited-state fold up to the nearest
        // node, which the shape rules bound at five elements; and a
        // resolution's palette-binding search, a one-off per resolution,
        // O(distance to the binding) (ruling R16.3 on #282).
        ("document/mutation/compose.rs", "tops_climbing"),
        ("document/mutation/inherited.rs", "state_of"),
        ("document/resolve.rs", "find_binding"),
    ];
    const CALLS: &[&str] = &[
        ".ancestors(",
        ".checked_insert_after(",
        ".checked_insert_before(",
        ".checked_prepend(",
        ".checked_append(",
    ];
    let mut files = Vec::new();
    for root in SCANNED {
        collect_recursively(&repo_path(root), ".rs", &mut files);
    }
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
        let rel = relative_to_repo(&file);
        let name = rel.strip_prefix("lib/baumhard/src/").unwrap_or(&rel).to_string();
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
            let calls = CALLS.iter().any(|call| code.contains(call));
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

/// What holds the engine's same-row re-attach to the arena's unchecked
/// insert ([`same_row_reattach_calls`]).
pub(crate) struct SameRowReattach {
    /// Each insert that checks for a cycle, or climb, in the body of
    /// `siblings::reattach_in_row`: `(repo-relative file, 1-based line,
    /// the line's text)`.
    pub(crate) checked: Vec<(String, usize, String)>,
    /// How many calls of the arena's unchecked `reattach_in_row` that
    /// body makes.
    pub(crate) unchecked: usize,
    /// Each `reattach_in_row` the crate's shipped code names outside
    /// that body that is not the engine function, in the same form.
    pub(crate) stray: Vec<(String, usize, String)>,
}

/// The engine's same-row re-attach, read from the source: whether
/// `siblings::reattach_in_row` — the one function the walker's settle,
/// `Tree::update` and a journal restoring an element to its row
/// re-arrange through — inserts through the arena's unchecked
/// `NodeId::reattach_in_row`, and through nothing that climbs (a
/// `checked_*` insert, `ancestors`, or the engine's `climb`); and
/// whether anything else in the crate's shipped code names that
/// unchecked insert. Ruling R16 on #282 and V1-A6 #290: a node put back
/// into the row it left cannot close a cycle, so its insert must not
/// pay the O(depth) check, and an insert that does not check may be
/// made only where the row is known to be the one the node left.
///
/// The scan reads syntax and matches the **identifier**
/// `reattach_in_row`, whatever spells it: a method call, a path
/// (`NodeId::reattach_in_row(..)`, a function pointer, a `use .. as`
/// alias of the type), a token inside a macro's arguments, or an item
/// of that name. What is the engine function rather than the arena's
/// method is allowed: its definition in `engine/siblings.rs`, a path
/// whose qualifier is `siblings` (`siblings::reattach_in_row(..)`, and
/// its `use`), and a bare call to it inside `engine/siblings.rs`. A
/// path with a qualified self (`<NodeId>::reattach_in_row`) is never the
/// engine function, which is a free `fn`; and since `siblings` must be
/// the module, a `use .. as siblings`, a `type siblings` or a
/// `mod siblings` anywhere but `engine/mod.rs` is stray in itself. The
/// arena module is read too: there, the method's definition in
/// `arena/link.rs` is the one mention allowed, so no wrapper in the
/// arena can launder the call for the crate.
///
/// It is a heuristic over spellings, and the step counts are its other
/// half: `test_same_row_rearrangement_is_linear_in_depth` counts every
/// node the arena's inserts of existing nodes go through and every step
/// of `engine::tree::climb`, so a climb on this path through either
/// shows there. A climb written by hand past both — a parent read in a
/// loop — is `test_the_engine_climbs_only_where_it_may`'s to find.
///
/// Cost: one read and one `syn` parse of every shipped `.rs` file under
/// `lib/baumhard/src/`.
pub(crate) fn same_row_reattach_calls() -> SameRowReattach {
    let mut files = Vec::new();
    collect_recursively(&repo_path("lib/baumhard/src"), ".rs", &mut files);
    files.sort();
    // Files a `#[cfg(test)] mod x;` names are test code, gated in the
    // file that declares them.
    let test_only = test_gated_module_files(&files);
    let mut out = SameRowReattach {
        checked: Vec::new(),
        unchecked: 0,
        stray: Vec::new(),
    };
    for file in files {
        let rel = relative_to_repo(&file);
        let in_tests = file.components().any(|c| c.as_os_str() == "tests");
        if in_tests || test_only.contains(&file) {
            continue;
        }
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        let source = production_source(&file, &text);
        let found = reattach_sites(&rel, &source);
        let lines: Vec<&str> = source.lines().collect();
        let site = |line: usize| {
            let code = lines.get(line.wrapping_sub(1)).map_or("", |l| l.trim());
            (rel.clone(), line, code.to_string())
        };
        out.unchecked += found.unchecked;
        out.checked.extend(found.checked.into_iter().map(site));
        out.stray.extend(found.stray.into_iter().map(site));
    }
    out
}

/// What [`reattach_sites`] found in one file, by 1-based line.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ReattachSites {
    /// Lines in the engine function's body that climb or check.
    pub(crate) checked: Vec<usize>,
    /// Calls of the arena's unchecked insert in that body.
    pub(crate) unchecked: usize,
    /// Lines that name `reattach_in_row` and are not the engine
    /// function.
    pub(crate) stray: Vec<usize>,
}

/// [`same_row_reattach_calls`] over one file: `rel` is its
/// repo-relative path, which says whether it is `engine/siblings.rs`,
/// `engine/mod.rs` or `arena/link.rs`, and `source` its text. Panics
/// when `source` does not parse.
pub(crate) fn reattach_sites(rel: &str, source: &str) -> ReattachSites {
    use syn::visit::{self, Visit};

    const NAME: &str = "reattach_in_row";

    struct Scan {
        /// The file is `engine/siblings.rs`.
        siblings: bool,
        /// The file is `engine/mod.rs`, which declares `mod siblings`.
        engine_mod: bool,
        /// The file is `arena/link.rs`, which defines the method.
        link: bool,
        in_body: bool,
        found: ReattachSites,
    }

    fn line(span: proc_macro2::Span) -> usize {
        span.start().line
    }

    /// Whether `tokens` hold the identifier `name` anywhere, groups
    /// included; the line of the first such token.
    fn token_line(tokens: &proc_macro2::TokenStream, name: &str) -> Option<usize> {
        let mut stack = vec![tokens.clone()];
        while let Some(tokens) = stack.pop() {
            for token in tokens {
                match token {
                    proc_macro2::TokenTree::Ident(ident) if ident == name => return Some(line(ident.span())),
                    proc_macro2::TokenTree::Group(group) => stack.push(group.stream()),
                    _ => {}
                }
            }
        }
        None
    }

    impl Scan {
        /// A path naming `reattach_in_row`: the engine function when its
        /// qualifier is `siblings`, or when it is bare inside
        /// `engine/siblings.rs` — and never with a qualified self
        /// (`<T>::`); otherwise the arena's method.
        fn path(&mut self, qself: bool, path: &syn::Path) {
            let segments: Vec<_> = path.segments.iter().collect();
            let Some(last) = segments.last() else {
                return;
            };
            if last.ident != NAME {
                return;
            }
            let engine = !qself
                && match segments.len() {
                    1 => self.siblings,
                    n => segments[n - 2].ident == "siblings",
                };
            if !engine {
                self.found.stray.push(line(last.ident.span()));
            }
        }
    }

    impl<'ast> Visit<'ast> for Scan {
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            if item.sig.ident == NAME {
                if !self.siblings {
                    self.found.stray.push(line(item.sig.ident.span()));
                    visit::visit_item_fn(self, item);
                    return;
                }
                let outer = std::mem::replace(&mut self.in_body, true);
                visit::visit_item_fn(self, item);
                self.in_body = outer;
                return;
            }
            visit::visit_item_fn(self, item);
        }

        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            if item.sig.ident == NAME && !self.link {
                self.found.stray.push(line(item.sig.ident.span()));
            }
            visit::visit_impl_item_fn(self, item);
        }

        fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
            if item.ident == "siblings" {
                self.found.stray.push(line(item.ident.span()));
            }
            visit::visit_item_type(self, item);
        }

        fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
            if item.ident == "siblings" && !self.engine_mod {
                self.found.stray.push(line(item.ident.span()));
            }
            visit::visit_item_mod(self, item);
        }

        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            let name = call.method.to_string();
            if name == NAME {
                if self.in_body {
                    self.found.unchecked += 1;
                } else {
                    self.found.stray.push(line(call.method.span()));
                }
            } else if self.in_body && (name.starts_with("checked_") || name == "ancestors" || name == "climb")
            {
                self.found.checked.push(line(call.method.span()));
            }
            visit::visit_expr_method_call(self, call);
        }

        fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
            if self.in_body {
                if let Some(last) = path.path.segments.last() {
                    let name = last.ident.to_string();
                    if name == "climb" || name == "ancestors" || name.starts_with("checked_") {
                        self.found.checked.push(line(last.ident.span()));
                    }
                }
            }
            self.path(path.qself.is_some(), &path.path);
            visit::visit_expr_path(self, path);
        }

        fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
            // A `use` of the engine function goes through `siblings`; a
            // `use` naming `reattach_in_row` any other way is not it.
            fn walk(tree: &syn::UseTree, through_siblings: bool, out: &mut Vec<usize>) {
                match tree {
                    syn::UseTree::Path(p) => walk(&p.tree, p.ident == "siblings", out),
                    syn::UseTree::Name(n) if n.ident == NAME && !through_siblings => {
                        out.push(line(n.ident.span()))
                    }
                    syn::UseTree::Rename(r) if r.ident == NAME && !through_siblings => {
                        out.push(line(r.ident.span()))
                    }
                    // Only the module may be called `siblings`.
                    syn::UseTree::Rename(r) if r.rename == "siblings" => out.push(line(r.rename.span())),
                    syn::UseTree::Group(g) => {
                        for item in &g.items {
                            walk(item, through_siblings, out);
                        }
                    }
                    _ => {}
                }
            }
            walk(tree, false, &mut self.found.stray);
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if let Some(at) = token_line(&mac.tokens, NAME) {
                self.found.stray.push(at);
            }
            if self.in_body {
                for name in ["climb", "ancestors"] {
                    if let Some(at) = token_line(&mac.tokens, name) {
                        self.found.checked.push(at);
                    }
                }
            }
            visit::visit_macro(self, mac);
        }
    }

    let parsed = syn::parse_file(source).unwrap_or_else(|e| panic!("{rel} must parse as Rust: {e}"));
    let mut scan = Scan {
        siblings: rel == "lib/baumhard/src/engine/siblings.rs",
        engine_mod: rel == "lib/baumhard/src/engine/mod.rs",
        link: rel == "lib/baumhard/src/arena/link.rs",
        in_body: false,
        found: ReattachSites::default(),
    };
    scan.visit_file(&parsed);
    scan.found.checked.sort();
    scan.found.checked.dedup();
    scan.found.stray.sort();
    scan.found.stray.dedup();
    scan.found
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
/// spellings: inside a `loop` or a `while`; inside a closure handed to
/// an iterator constructor or adapter that feeds the closure its own
/// last result (`FEEDBACK`); or inside a `for` body or any
/// closure that assigns a plain variable — or through a `*` deref —
/// that the read names (in its receiver or its arguments). As `(the
/// enclosing function's name, 1-based line)`. A closure counts where it
/// is written. Panics when `source` does not parse, naming `file`.
fn parent_reads_in_loops(file: &Path, source: &str) -> Vec<(String, usize)> {
    use std::collections::BTreeSet;
    use syn::visit::{self, Visit};

    /// The iterator constructors and adapters whose closure is handed
    /// its own last result, or keeps state from call to call by its
    /// nature: a parent read there is a climb.
    const FEEDBACK: &[&str] = &[
        "successors",
        "from_fn",
        "repeat_with",
        "scan",
        "fold",
        "rfold",
        "try_fold",
        "try_rfold",
        "reduce",
        "try_reduce",
    ];

    struct Finder {
        function: String,
        loops: usize,
        /// For each enclosing `for` body and closure, the plain
        /// variables it assigns.
        cursors: Vec<BTreeSet<String>>,
        found: Vec<(String, usize)>,
    }

    /// The plain variables `expr` assigns anywhere inside it, directly
    /// (`at = p`) or through a deref (`*at = p`).
    fn assigned(expr: &syn::Expr) -> BTreeSet<String> {
        struct Assigned(BTreeSet<String>);
        impl<'ast> Visit<'ast> for Assigned {
            fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
                let mut left = &*assign.left;
                while let syn::Expr::Unary(syn::ExprUnary {
                    op: syn::UnOp::Deref(_),
                    expr,
                    ..
                }) = left
                {
                    left = expr;
                }
                if let syn::Expr::Path(path) = left {
                    if let Some(ident) = path.path.get_ident() {
                        self.0.insert(ident.to_string());
                    }
                }
                visit::visit_expr_assign(self, assign);
            }
        }
        let mut found = Assigned(BTreeSet::new());
        found.visit_expr(expr);
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
            let body = syn::Expr::Block(syn::ExprBlock {
                attrs: Vec::new(),
                label: None,
                block: expr.body.clone(),
            });
            self.cursors.push(assigned(&body));
            self.visit_block(&expr.body);
            self.cursors.pop();
        }

        fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
            self.cursors.push(assigned(&closure.body));
            visit::visit_expr_closure(self, closure);
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
        assert_eq!(found("let _ = (0..n).try_fold(t, |c, _| arena[c].parent());"), 1);
        assert_eq!(
            found("let _ = (0..n).fold(t, |c, _| arena[c].parent().unwrap_or(c));"),
            1
        );
        assert_eq!(
            found("let _ = std::iter::repeat(()).map_while(|_| { let p = arena[h].parent()?; h = p; Some(()) }).count();"),
            1
        );
        assert_eq!(
            found("ids.iter().for_each(|_| { if let Some(p) = arena[at].parent() { at = p; } });"),
            1
        );
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

    /// **The engine's same-row re-attach climbs nothing, and is the only
    /// caller of the arena's unchecked insert**
    /// ([`same_row_reattach_calls`]). The body of
    /// `siblings::reattach_in_row` inserts through the arena's unchecked
    /// `reattach_in_row` and through no checked insert or climb; no other
    /// shipped file of the crate names that unchecked insert, which could
    /// hang a node below itself under any parent but the one it left.
    #[test]
    fn test_the_same_row_reattach_is_the_unchecked_insert() {
        let calls = same_row_reattach_calls();
        assert!(
            calls.unchecked >= 1,
            "precondition: the scan reads the body of siblings::reattach_in_row, which \
             re-attaches through the arena's unchecked insert (found {})",
            calls.unchecked
        );
        let show = |sites: &[(String, usize, String)]| {
            sites
                .iter()
                .map(|(file, line, code)| format!("  {file}:{line}   {code}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(
            calls.checked.is_empty(),
            "the same-row re-attach checks for a cycle or climbs, an O(depth) cost a node \
             put back into its own row cannot need (ruling R16 on #282):\n{}",
            show(&calls.checked)
        );
        assert!(
            calls.stray.is_empty(),
            "the arena's unchecked insert named outside siblings::reattach_in_row, where \
             nothing holds the row to the one the node left:\n{}",
            show(&calls.stray)
        );
    }

    /// [`reattach_sites`] reads every spelling of the unchecked insert,
    /// and of a climb in the engine function's body — each control of
    /// the scan, kept in the suite.
    #[test]
    fn test_reattach_sites_reads_every_spelling() {
        const SIBLINGS: &str = "lib/baumhard/src/engine/siblings.rs";
        const ELSEWHERE: &str = "lib/baumhard/src/engine/structure.rs";
        let stray = |body: &str| {
            reattach_sites(ELSEWHERE, &format!("fn f() {{ {body} }}"))
                .stray
                .len()
        };
        // The arena's method, every way it can be named.
        assert_eq!(
            stray("let _ = n.reattach_in_row(p, None, arena);"),
            1,
            "a method call"
        );
        assert_eq!(
            stray("let _ = NodeId::reattach_in_row(n, p, None, arena);"),
            1,
            "UFCS"
        );
        assert_eq!(
            stray("let _ = crate::arena::NodeId::reattach_in_row(n, p, None, arena);"),
            1,
            "UFCS, qualified"
        );
        assert_eq!(
            stray("let f = NodeId::reattach_in_row; let _ = f(n, p, None, arena);"),
            1,
            "a fn pointer"
        );
        assert_eq!(
            stray("use crate::arena::NodeId as Id; let _ = Id::reattach_in_row(n, p, None, arena);"),
            1,
            "through a `use .. as` alias of the type"
        );
        assert_eq!(
            stray("log::info!(\"{:?}\", n.reattach_in_row(p, None, arena));"),
            1,
            "in a macro"
        );
        assert_eq!(
            reattach_sites(ELSEWHERE, "fn reattach_in_row() {}").stray.len(),
            1,
            "a function of that name elsewhere"
        );
        assert_eq!(
            reattach_sites(
                SIBLINGS,
                "fn f() { let _ = <NodeId>::reattach_in_row(n, p, None, arena); }"
            )
            .stray
            .len(),
            1,
            "a qualified self, even in siblings.rs"
        );
        assert_eq!(
            stray(
                "use crate::arena::NodeId as siblings; let _ = siblings::reattach_in_row(n, p, None, arena);"
            ),
            1,
            "the type renamed `siblings`: the call through it reads as the engine function, so the \
             rename itself is stray"
        );
        assert_eq!(
            reattach_sites(ELSEWHERE, "type siblings = crate::arena::NodeId;")
                .stray
                .len(),
            1,
            "a type called `siblings`"
        );
        assert_eq!(
            reattach_sites(ELSEWHERE, "mod siblings {}").stray.len(),
            1,
            "a module called `siblings` outside engine/mod.rs"
        );
        assert_eq!(
            reattach_sites("lib/baumhard/src/engine/mod.rs", "mod siblings;")
                .stray
                .len(),
            0,
            "the engine's own `mod siblings`"
        );
        // The arena: the method's definition, and nothing else.
        const LINK: &str = "lib/baumhard/src/arena/link.rs";
        let method = "impl NodeId { pub(crate) fn reattach_in_row(self) {} }";
        assert_eq!(
            reattach_sites(LINK, method).stray.len(),
            0,
            "the method's definition"
        );
        assert_eq!(
            reattach_sites(
                LINK,
                &format!("{method} pub(crate) fn put_back(n: NodeId) {{ n.reattach_in_row(); }}")
            )
            .stray
            .len(),
            1,
            "a wrapper in the arena"
        );
        assert_eq!(
            reattach_sites("lib/baumhard/src/arena/mod.rs", method)
                .stray
                .len(),
            1,
            "a definition anywhere but arena/link.rs"
        );
        // The engine function, which is allowed.
        assert_eq!(
            stray("siblings::reattach_in_row(arena, p, n, None);"),
            0,
            "the engine function"
        );
        assert_eq!(
            stray("crate::engine::siblings::reattach_in_row(arena, p, n, None);"),
            0,
            "the engine function, qualified"
        );
        assert_eq!(
            reattach_sites(ELSEWHERE, "use crate::engine::siblings::reattach_in_row;")
                .stray
                .len(),
            0,
            "a `use` of the engine function"
        );
        // The engine function's body.
        let body = |line: &str| {
            reattach_sites(
                SIBLINGS,
                &format!(
                    "pub(crate) fn reattach_in_row<T>(arena: &mut Arena<T>, parent: NodeId, node: NodeId, \
                     prev: Option<NodeId>) {{ {line} let _ = node.reattach_in_row(parent, prev, arena); }}"
                ),
            )
        };
        let clean = body("");
        assert_eq!(
            (clean.unchecked, clean.checked.len(), clean.stray.len()),
            (1, 0, 0),
            "the shipped shape"
        );
        for planted in [
            "let _ = crate::engine::tree::climb(arena, parent).count();",
            "let _ = climb(arena, parent).count();",
            "let _ = parent.ancestors(arena).count();",
            "let _ = parent.checked_prepend(node, arena);",
            "let _ = NodeId::checked_prepend(parent, node, arena);",
        ] {
            assert_eq!(
                body(planted).checked.len(),
                1,
                "a climb or check planted: {planted}"
            );
        }
    }
}
