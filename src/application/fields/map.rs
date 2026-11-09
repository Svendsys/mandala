// SPDX-License-Identifier: MPL-2.0

//! The map's theme and defaults (design §5.3, §5.4, D21).
//!
//! - **The theme** stores its variants and the name of the active one;
//!   switching variants is one `ThemeOp::SetActive`, and a variable's
//!   value in one variant one `ThemeOp::SetValue`. A theme value is a
//!   literal color, as the file's is.
//! - **The defaults** are one element, replaced whole
//!   (`DefaultsOp::Replace`) with the fields a `defaults` line names
//!   changed and every other field as it was. Each field is named
//!   `<group>.<field>` with the file's names (`frame.fit`,
//!   `portal_end.border_t`), and read in the console's spelling of its
//!   value ([`values`]). A frame's and a ring's `size` is a fraction of
//!   the node's shorter side; every other `size` is in points.

use baumhard::document::element::DocKind;
use baumhard::document::element::{DefaultsData, FlowStyle, RingLayoutStyle, TextStyle};
use baumhard::document::ids::{self, Document};
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DefaultsOp, DocMutation, ThemeOp};
use baumhard::document::values::{ColorRef, FontSize, VarId};

use super::values::{self, or_reset};
use super::Steps;
use crate::application::document::MindMapDocument;

/// The index of the theme variant `name`.
fn variant(doc: &Document, name: &str) -> Result<u16, String> {
    let theme = doc.theme();
    theme
        .variants
        .iter()
        .position(|variant| variant.name == name)
        .and_then(|at| u16::try_from(at).ok())
        .ok_or_else(|| {
            let names: Vec<&str> = theme.variants.iter().map(|v| v.name.as_str()).collect();
            match names.is_empty() {
                true => format!("theme: '{name}' is not a variant; this map's theme has none"),
                false => format!(
                    "theme: '{name}' is not a variant of this map's theme: {}",
                    names.join(", ")
                ),
            }
        })
}

/// One walk of `mutation` on the theme element.
fn on_theme(doc: &Document, mutation: DocMutation) -> Steps {
    let (theme, _) = ids::theme(doc.tree()).ok_or("theme: the map holds no theme")?;
    compose::set(doc, DocKind::Theme, vec![mutation], &[theme]).map_err(|e| e.to_string())
}

/// The steps that make variant `name` the active one.
pub(crate) fn activate(doc: &MindMapDocument, name: &str) -> Steps {
    let d = doc.doc();
    on_theme(d, DocMutation::Theme(ThemeOp::SetActive(variant(d, name)?)))
}

/// The steps that set theme variables to colors in variant `in_variant`
/// (the active one when `None`): `values` pairs a variable's name with
/// a literal color.
pub(crate) fn set_theme(
    doc: &MindMapDocument,
    values_given: &[(&str, &str)],
    in_variant: Option<&str>,
) -> Steps {
    let d = doc.doc();
    let theme = d.theme();
    let variant = match in_variant {
        Some(name) => variant(d, name)?,
        None => theme.active,
    };
    let (element, _) = ids::theme(d.tree()).ok_or("theme: the map holds no theme")?;
    let mut mutations = Vec::with_capacity(values_given.len());
    for &(name, text) in values_given {
        let var = theme
            .variables
            .iter()
            .position(|known| known == name)
            .and_then(|at| u16::try_from(at).ok())
            .map(VarId)
            .ok_or_else(|| format!("theme: '{name}' is not a variable of this map's theme"))?;
        let value = match values::color(d, text)? {
            Some(ColorRef::Rgba(rgba)) => rgba,
            _ => {
                return Err(format!(
                    "theme: '{text}' is not a literal color: a theme value is `#rrggbb[aa]`"
                ))
            }
        };
        mutations.push(DocMutation::Theme(ThemeOp::SetValue { variant, var, value }));
    }
    if mutations.is_empty() {
        return Err("theme set: give <variable>=<color>".to_string());
    }
    compose::set(d, DocKind::Theme, mutations, &[element]).map_err(|e| e.to_string())
}

/// The groups `defaults` names, as the file names them.
pub(crate) const GROUPS: [&str; 9] = [
    "node",
    "section",
    "frame",
    "ring",
    "connector",
    "link",
    "label",
    "portal_end",
    "span",
];

/// Set text field `field` of `style` to `value`; `relative` reads a
/// size as a fraction of the node's shorter side. `Ok(false)` for a
/// field a text style does not have.
fn text(
    doc: &Document,
    style: &mut TextStyle,
    field: &str,
    value: &str,
    relative: bool,
) -> Result<bool, String> {
    match field {
        "font" => style.font = values::font(value)?,
        "size" => {
            style.size = or_reset(value, values::number)?.map(|v| {
                if relative {
                    FontSize::Rel(v)
                } else {
                    FontSize::Pt(v)
                }
            })
        }
        "color" => style.color = values::color(doc, value)?,
        "weight" => style.weight = or_reset(value, values::weight)?,
        "italic" => style.italic = or_reset(value, values::boolean)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Set ring-layout field `field` of `layout`. `Ok(false)` for another.
fn layout(layout: &mut RingLayoutStyle, field: &str, value: &str) -> Result<bool, String> {
    match field {
        "span" => layout.span = or_reset(value, values::line_span)?,
        "sides" => layout.sides = or_reset(value, values::sides)?,
        "spacing" => layout.spacing = or_reset(value, values::number)?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Set path-flow field `field` of `flow`. `Ok(false)` for another.
fn flow(doc: &Document, flow: &mut FlowStyle, field: &str, value: &str) -> Result<bool, String> {
    match field {
        "fit" => flow.fit = or_reset(value, values::fit)?,
        "spacing" => flow.spacing = or_reset(value, values::number)?,
        "size" => flow.size = or_reset(value, values::number)?,
        "color" => flow.color = values::color(doc, value)?,
        "content" => flow.content = or_reset(value, |text| Ok(text.to_string()))?,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Set `key` (`<group>.<field>`) of `defaults` to `value`, read in its
/// field's spelling, or `reset` to unset it.
pub(crate) fn set_default(
    doc: &Document,
    defaults: &mut DefaultsData,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let unknown = || {
        format!(
            "defaults: '{key}' is not a default: <group>.<field>, the groups {}",
            GROUPS.join(", ")
        )
    };
    let (group, field) = key.split_once('.').ok_or_else(unknown)?;
    let num = |value: &str| or_reset(value, values::number);
    let known = match group {
        "node" if field == "shape" => {
            defaults.node.shape = or_reset(value, values::shape)?;
            true
        }
        "node" => text(doc, &mut defaults.node.text, field, value, false)?,
        "section" => text(doc, &mut defaults.section, field, value, false)?,
        "span" => text(doc, &mut defaults.span, field, value, false)?,
        "frame" => {
            let f = &mut defaults.frame;
            match field {
                "visible" => f.visible = or_reset(value, values::boolean)?,
                "start" => f.start = or_reset(value, values::start)?,
                "direction" => f.direction = or_reset(value, values::direction)?,
                "fit" => f.fit = or_reset(value, values::fit)?,
                "gap" => f.gap = num(value)?,
                "inset" => f.inset = num(value)?,
                _ => {
                    if !layout(&mut f.ring_layout, field, value)? {
                        return text(doc, &mut f.text, field, value, true)?
                            .then_some(())
                            .ok_or_else(unknown);
                    }
                }
            }
            true
        }
        "ring" => {
            let r = &mut defaults.ring;
            match field {
                "fit" => {
                    r.fit = or_reset(value, values::fit)?;
                    true
                }
                _ => layout(&mut r.ring_layout, field, value)? || text(doc, &mut r.text, field, value, true)?,
            }
        }
        "connector" => flow(doc, &mut defaults.connector, field, value)?,
        "link" => flow(doc, &mut defaults.link, field, value)?,
        "label" => {
            let l = &mut defaults.label;
            match field {
                "t" => l.t = num(value)?,
                "offset" => l.offset = num(value)?,
                _ => {
                    return text(doc, &mut l.text, field, value, false)?
                        .then_some(())
                        .ok_or_else(unknown)
                }
            }
            true
        }
        "portal_end" => {
            let p = &mut defaults.portal_end;
            match field {
                "border_t" => p.border_t = num(value)?,
                "offset" => p.offset = num(value)?,
                "color" => p.color = values::color(doc, value)?,
                "size" => p.size = num(value)?,
                "content" => p.content = or_reset(value, |text| Ok(text.to_string()))?,
                _ => return Err(unknown()),
            }
            true
        }
        _ => false,
    };
    known.then_some(()).ok_or_else(unknown)
}

/// The steps that replace the defaults with every `(key, value)` of
/// `changes` set ([`set_default`]) and every other field as it is.
pub(crate) fn defaults(doc: &MindMapDocument, changes: &[(&str, &str)]) -> Steps {
    let d = doc.doc();
    if changes.is_empty() {
        return Err("defaults: give <group>.<field>=<value>".to_string());
    }
    let mut defaults = d.defaults().clone();
    for &(key, value) in changes {
        set_default(d, &mut defaults, key, value)?;
    }
    let (element, _) = ids::defaults(d.tree()).ok_or("defaults: the map holds no defaults")?;
    compose::set(
        d,
        DocKind::Defaults,
        vec![DocMutation::Defaults(DefaultsOp::Replace(Box::new(defaults)))],
        &[element],
    )
    .map_err(|e| e.to_string())
}
