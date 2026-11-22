// SPDX-License-Identifier: MPL-2.0

//! The installed font families a map names, and what to do when this
//! machine lacks one (#329).
//!
//! A map may name a family no compiled face bears: an installed one,
//! held by its name ([`FontRef::Installed`](crate::document::values::FontRef::Installed)). Opening the map never
//! fails for it. Where the machine has the family, its text is drawn in
//! its faces; where it does not, in the **stand-in**, the default face
//! ([`STAND_IN`](crate::document::fonts::STAND_IN)), at the span's own weight and slant, until the user
//! decides — installing the font, which a rescan picks up, or picking a
//! replacement, which [`replace_steps`](crate::document::fonts::replace_steps) makes one undoable edit. Saving
//! writes a family nobody replaced back as it was named.
//!
//! [`missing_families`](crate::document::fonts::missing_families) is the report: one walk of the tree, nothing
//! stored, so it is right after any edit, undo or redo without an index
//! to keep. It asks the font system which installed families this
//! machine has only when the map names one, so a map that names only
//! compiled families costs no host font scan.
//!
//! **What a replacement does not rewrite.** The edits and presentations
//! a map defines are not changed while it is open (design §5.7), so a
//! family one of them names is reported ([`NamedFamily::definitions`](crate::document::fonts::NamedFamily::definitions))
//! and left as it is; running it sets the family again. Whether the
//! replacement should rewrite them too is the owner's decision on #329.

use crate::arena::NodeId;
use crate::document::element::{DefaultsData, DocKind, TextStyle};
use crate::document::ids::{defaults, Document};
use crate::document::mutation::compose;
use crate::document::mutation::error::MutationError;
use crate::document::mutation::ops::{DefaultsOp, DocMutation, TextOp};
use crate::document::mutation::step::DocStep;
use crate::document::values::{FamilyName, FontRef};
use crate::font::fallback::DEFAULT_FACE;

/// The face a missing family's text is drawn in until the user installs
/// the family or picks a replacement: the default face, `Inter Variable`,
/// at the span's own weight and slant (its `wght` axis spans every
/// weight; an italic span is skewed, as default text is).
pub const STAND_IN: &str = DEFAULT_FACE.1;

/// An installed family a map names, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedFamily {
    /// The family name.
    pub family: FamilyName,
    /// How many of the map's elements name it in their own text style,
    /// each of the defaults' text styles counted as one.
    pub elements: usize,
    /// The edits and presentations that name it, as `edit `id`` or
    /// `presentation `id``, in the order the map lists them. A
    /// replacement does not rewrite them (the module docs).
    pub definitions: Vec<String>,
}

/// A family a map names that this machine lacks: its text is drawn in
/// [`STAND_IN`] until the user installs it or picks a replacement.
pub type MissingFamily = NamedFamily;

/// Every installed family `doc` names, sorted by name: what a map asks of
/// the machine it is opened on. Reads no font system.
///
/// Cost: one walk of the tree (iterative, no work per depth), one name
/// comparison per element that names an installed family against the
/// families found so far, and one walk of each definition.
pub fn installed_families(doc: &Document) -> Vec<NamedFamily> {
    let mut found: Vec<NamedFamily> = Vec::new();
    let mut count = |family: &FamilyName| match found.iter_mut().find(|named| named.family == *family) {
        Some(named) => named.elements += 1,
        None => found.push(NamedFamily {
            family: family.clone(),
            elements: 1,
            definitions: Vec::new(),
        }),
    };
    for id in doc.tree().descendants() {
        let Some(element) = doc.get(id) else { continue };
        if let Some(family) = element
            .text_style()
            .and_then(|style| style.font.as_ref())
            .and_then(FontRef::installed)
        {
            count(family);
        }
    }
    for style in defaults_styles(doc.defaults()) {
        if let Some(family) = style.font.as_ref().and_then(FontRef::installed) {
            count(family);
        }
    }
    let definitions = doc.definitions();
    let mut named_by = |owner: String, name: &str| {
        let Ok(FontRef::Installed(family)) = crate::document::format::font(name) else {
            return;
        };
        let at = match found.iter().position(|named| named.family == family) {
            Some(at) => at,
            None => {
                found.push(NamedFamily {
                    family,
                    elements: 0,
                    definitions: Vec::new(),
                });
                found.len() - 1
            }
        };
        if !found[at].definitions.contains(&owner) {
            found[at].definitions.push(owner);
        }
    };
    for edit in &definitions.edits {
        edit.fonts(|name| named_by(format!("edit `{}`", edit.id), name));
    }
    for presentation in &definitions.presentations {
        presentation.fonts(|name| named_by(format!("presentation `{}`", presentation.id), name));
    }
    found.sort_by(|a, b| a.family.cmp(&b.family));
    found
}

/// Every installed family `doc` names that this machine lacks, sorted by
/// name: the families whose text is drawn in [`STAND_IN`], what the
/// missing-font prompt lists and IPC's `query.fonts` reports. Empty for a
/// map that names only compiled families, which costs no font system.
/// In the browser, which reads no installed font, every installed family
/// is missing.
///
/// Cost: [`installed_families`], and when the map names one, the font
/// system's family index ([`installed_family_loaded`](crate::font::fonts::installed_family_loaded))
/// — the host's font scan, if nothing has paid for it yet.
pub fn missing_families(doc: &Document) -> Vec<MissingFamily> {
    let mut named = installed_families(doc);
    if !named.is_empty() {
        named.retain(|named| !crate::font::fonts::installed_family_loaded(named.family.as_str()));
    }
    named
}

/// The steps of the one edit that rewrites every reference `doc`'s tree
/// makes to the installed family `from` — every element's own text style
/// and each of the defaults' — to `to`, a compiled face or another
/// family a map can name. One request of these steps is one undo entry,
/// and its undo restores every reference.
///
/// The steps are one walk per element kind that names `from` (a
/// [`compose::set`] of `SetFont(to)` on its elements) and one replace of
/// the defaults where they name it. The map's edits and presentations
/// are not among them (the module docs); a kind added here is one more
/// group of steps.
///
/// Errors: [`MutationError::AnyFont`] for a `to` no file can name — the
/// default face. Empty when `from` is named nowhere, or `to` is `from`.
///
/// Cost: one walk of the tree, and one allocation per element that names
/// `from`.
pub fn replace_steps(doc: &Document, from: &FamilyName, to: &FontRef) -> Result<Vec<DocStep>, MutationError> {
    if !crate::document::place::nameable_font(to) {
        return Err(MutationError::AnyFont {
            at: Default::default(),
        });
    }
    if to.installed() == Some(from) {
        return Ok(Vec::new());
    }
    let names = |style: &TextStyle| style.font.as_ref().and_then(FontRef::installed) == Some(from);
    let mut groups: Vec<(DocKind, Vec<NodeId>)> = Vec::new();
    for id in doc.tree().descendants() {
        let Some(element) = doc.get(id) else { continue };
        if !element.text_style().is_some_and(names) {
            continue;
        }
        let kind = element.kind();
        match groups.iter_mut().find(|(group, _)| *group == kind) {
            Some((_, ids)) => ids.push(id),
            None => groups.push((kind, vec![id])),
        }
    }
    let mut steps = Vec::new();
    for (kind, ids) in groups {
        let Some(mutation) = TextOp::SetFont(Some(to.clone())).on(kind) else {
            continue;
        };
        steps.extend(compose::set(doc, kind, vec![mutation], &ids)?);
    }
    if let Some((id, data)) = defaults(doc.tree()) {
        if defaults_styles(data).any(names) {
            let mut replaced = data.clone();
            for style in defaults_styles_mut(&mut replaced) {
                if names(style) {
                    style.font = Some(to.clone());
                }
            }
            steps.extend(compose::set(
                doc,
                DocKind::Defaults,
                vec![DocMutation::Defaults(DefaultsOp::Replace(Box::new(replaced)))],
                &[id],
            )?);
        }
    }
    Ok(steps)
}

/// The defaults' text styles: the node's, a section's, a frame's, a
/// ring's, a label's and a span's.
fn defaults_styles(defaults: &DefaultsData) -> impl Iterator<Item = &TextStyle> {
    [
        &defaults.node.text,
        &defaults.section,
        &defaults.frame.text,
        &defaults.ring.text,
        &defaults.label.text,
        &defaults.span,
    ]
    .into_iter()
}

/// [`defaults_styles`], to change.
fn defaults_styles_mut(defaults: &mut DefaultsData) -> impl Iterator<Item = &mut TextStyle> {
    [
        &mut defaults.node.text,
        &mut defaults.section,
        &mut defaults.frame.text,
        &mut defaults.ring.text,
        &mut defaults.label.text,
        &mut defaults.span,
    ]
    .into_iter()
}
