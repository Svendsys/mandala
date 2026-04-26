// SPDX-License-Identifier: MPL-2.0

//! `border` execute path: positional dispatch + atomic kv apply.
//!
//! The kv form parses every recognised key into a
//! [`BorderConfigEdits`] up front, validates each value against
//! its typed parser, then hands the whole bundle to
//! [`MindMapDocument::set_node_border_config`] per selected node.
//! Validation failures abort before any node is mutated.
//!
//! ## Why parse-then-dispatch instead of `apply_kvs` / capability traits
//!
//! The `color` verb dispatches per-kv through `apply_kvs` against
//! the capability traits on `TargetView` (`HasBgColor`,
//! `HasTextColor`, `HasBorderColor`) because each kv targets a
//! *different* trait channel — `bg=#x text=#y border=#z` writes
//! three independent fields, each of which can have its own
//! "not applicable to this selection" answer.
//!
//! `border` and `font` (see `commands/font.rs`) are
//! single-channel verbs: every kv targets the same per-node
//! `GlyphBorderConfig` (or, for `font`, the same edge / label /
//! portal channel). The right shape there is to parse every kv
//! up front, validate the bundle, and hand it to one document
//! setter that applies the whole bundle atomically — exactly
//! what this file does. A `HasBorder` trait would be
//! multi-method, only ever implemented on `TargetView::Node`
//! with `NotApplicable` everywhere else, and would force one
//! trait call per kv per node which breaks the atomic-apply
//! invariant the verb relies on for parse-error rejection.
//!
//! Recorded so a future reviewer doesn't relitigate this. See
//! `apply_kvs` (`src/application/console/traits/dispatch.rs`)
//! and `font.rs::execute_font` for the two precedents.

use baumhard::mindmap::border::PaletteField;
use baumhard::mindmap::border_pattern::SidePattern;

use crate::application::console::parser::Args;
use crate::application::console::traits::ColorValue;
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{
    BorderConfigEdits, BorderEditOutcome, BorderFieldEdit, BorderSide, SelectionState,
};

use super::show::execute_border_show;

pub fn execute_border(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    if let Some(verb) = args.positional(0) {
        match verb {
            "on" => return apply_visible_only(eff, true),
            "off" => return apply_visible_only(eff, false),
            "show" => return execute_border_show(args, eff),
            "reset" => return apply_reset(eff),
            other if !other.contains('=') => {
                return ExecResult::err(format!(
                    "border: unknown subverb '{}'; use \
                     'on', 'off', 'show', 'reset', or kv form",
                    other
                ));
            }
            _ => {}
        }
    }

    // kv form: collect every recognised key, parse + validate
    // before any mutation. An unknown key aborts with a
    // pointer-style error.
    let mut edits = BorderConfigEdits::default();
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        saw_any = true;
        if let Err(e) = stage_kv(&mut edits, k, v) {
            return ExecResult::err(e);
        }
    }
    if !saw_any {
        return ExecResult::err(
            "usage: border on|off|show|reset | border <key>=<value> …",
        );
    }
    apply_edits(eff, edits)
}

fn apply_visible_only(eff: &mut ConsoleEffects, on: bool) -> ExecResult {
    let ids = match nodes_in_selection(&eff.document.selection) {
        Ok(ids) => ids,
        Err(e) => return e,
    };
    let mut changed = 0usize;
    for id in &ids {
        if eff.document.set_node_border_visible(id, on) {
            changed += 1;
        }
    }
    if changed == 0 {
        return ExecResult::ok_msg(format!(
            "border: already {}",
            if on { "on" } else { "off" }
        ));
    }
    ExecResult::ok_msg(format!(
        "border {} on {} node(s)",
        if on { "on" } else { "off" },
        changed
    ))
}

fn apply_reset(eff: &mut ConsoleEffects) -> ExecResult {
    let mut edits = BorderConfigEdits::default();
    edits.clear = true;
    apply_edits(eff, edits)
}

fn apply_edits(eff: &mut ConsoleEffects, edits: BorderConfigEdits) -> ExecResult {
    let ids = match nodes_in_selection(&eff.document.selection) {
        Ok(ids) => ids,
        Err(e) => return e,
    };
    let mut changed = 0usize;
    let mut auto_promoted: Option<String> = None;
    for id in &ids {
        let outcome: BorderEditOutcome =
            eff.document.set_node_border_config(id, edits.clone());
        if outcome.changed {
            changed += 1;
        }
        if outcome.preset_auto_promoted && auto_promoted.is_none() {
            auto_promoted = outcome.requested_preset.clone();
        }
    }
    if changed == 0 {
        return ExecResult::ok_msg("border: no change");
    }
    // Surface auto-promotion exactly once per command invocation,
    // not once per affected node — the same edit applies to every
    // selected node so the message would be redundant. Only the
    // first promoted node's `requested_preset` is reported; every
    // other node received the same edit struct, so the value is
    // necessarily the same.
    let main = format!("border applied to {} node(s)", changed);
    match auto_promoted {
        Some(name) => ExecResult::lines(vec![
            main,
            format!(
                "note: preset='{}' auto-promoted to 'custom' \
                 (a side or corner glyph was set; non-custom presets \
                 ignore the per-node glyph override)",
                name
            ),
        ]),
        None => ExecResult::ok_msg(main),
    }
}

/// Resolve the current selection into a list of node ids, or an
/// `ExecResult::Err` describing why it can't apply (no selection /
/// non-node selection). Edge-adjacent selections surface a single
/// "not applicable" line — borders are node-only.
fn nodes_in_selection(sel: &SelectionState) -> Result<Vec<String>, ExecResult> {
    match sel {
        SelectionState::Single(id) => Ok(vec![id.clone()]),
        SelectionState::Multi(ids) => Ok(ids.clone()),
        SelectionState::None => Err(ExecResult::err(
            "border: no selection (select a node first)",
        )),
        SelectionState::Edge(_) => Err(ExecResult::err(
            "border: not applicable to edges",
        )),
        SelectionState::EdgeLabel(_) => Err(ExecResult::err(
            "border: not applicable to edge labels",
        )),
        SelectionState::PortalLabel(_) => Err(ExecResult::err(
            "border: not applicable to portal labels",
        )),
        SelectionState::PortalText(_) => Err(ExecResult::err(
            "border: not applicable to portal text",
        )),
    }
}

/// Parse one `key=value` pair into the appropriate slot on
/// `edits`. Returns the same error string the user sees in the
/// console — kept verbatim so `border top="a)"` reports the parser
/// output ("unmatched ')'…") with a `top: ` prefix.
fn stage_kv(
    edits: &mut BorderConfigEdits,
    key: &str,
    value: &str,
) -> Result<(), String> {
    match key {
        "preset" => stage_preset(edits, value),
        "font" => stage_font(edits, value),
        "size" => stage_size(edits, value),
        "color" => stage_color(edits, value),
        "padding" => stage_padding(edits, value),
        "palette" => stage_palette(edits, value),
        "field" => stage_field(edits, value),
        "top" => edits.with_side_pattern(BorderSide::Top, value),
        "bottom" => edits.with_side_pattern(BorderSide::Bottom, value),
        "left" => edits.with_side_pattern(BorderSide::Left, value),
        "right" => edits.with_side_pattern(BorderSide::Right, value),
        "tl" => stage_corner_or_err(&mut edits.corner_top_left, "tl", value),
        "tr" => stage_corner_or_err(&mut edits.corner_top_right, "tr", value),
        "bl" => stage_corner_or_err(&mut edits.corner_bottom_left, "bl", value),
        "br" => stage_corner_or_err(&mut edits.corner_bottom_right, "br", value),
        other => Err(format!(
            "unknown key '{}'; valid keys: {}",
            other,
            super::KEYS.join(" | ")
        )),
    }
}

fn stage_preset(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    let v = value.to_ascii_lowercase();
    if !super::PRESETS.iter().any(|p| *p == v) {
        return Err(format!(
            "preset '{}' unknown; pick one of {}",
            value,
            super::PRESETS.join(" | ")
        ));
    }
    edits.preset = BorderFieldEdit::Set(v);
    Ok(())
}

fn stage_font(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    if value == "off" || value.is_empty() {
        edits.font = BorderFieldEdit::Clear;
        return Ok(());
    }
    if baumhard::font::fonts::app_font_by_family(value).is_none() {
        return Err(format!(
            "font '{}' is not a loaded font; try `font list`",
            value
        ));
    }
    edits.font = BorderFieldEdit::Set(value.to_string());
    Ok(())
}

fn stage_size(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    let pt = parse_pt("size", value)?;
    edits.font_size_pt = BorderFieldEdit::Set(pt);
    Ok(())
}

fn stage_padding(
    edits: &mut BorderConfigEdits,
    value: &str,
) -> Result<(), String> {
    let pt = parse_pt("padding", value)?;
    edits.padding = BorderFieldEdit::Set(pt);
    Ok(())
}

fn stage_color(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    let cv =
        ColorValue::parse(value).map_err(|e| format!("color: {}", e))?;
    edits.color = match cv {
        ColorValue::Reset => BorderFieldEdit::Clear,
        other => BorderFieldEdit::Set(
            other
                .as_model_string()
                .ok_or_else(|| "color: unexpected reset variant".to_string())?,
        ),
    };
    Ok(())
}

fn stage_palette(
    edits: &mut BorderConfigEdits,
    value: &str,
) -> Result<(), String> {
    if value.eq_ignore_ascii_case("off") || value.is_empty() {
        edits.color_palette = BorderFieldEdit::Clear;
        return Ok(());
    }
    edits.color_palette = BorderFieldEdit::Set(value.to_string());
    Ok(())
}

fn stage_field(
    edits: &mut BorderConfigEdits,
    value: &str,
) -> Result<(), String> {
    if value.eq_ignore_ascii_case("off") || value.is_empty() {
        edits.color_palette_field = BorderFieldEdit::Clear;
        return Ok(());
    }
    let lower = value.to_ascii_lowercase();
    let parsed = match lower.as_str() {
        "frame" => PaletteField::Frame,
        "background" => PaletteField::Background,
        "text" => PaletteField::Text,
        "title" => PaletteField::Title,
        other => {
            return Err(format!(
                "field '{}' unknown; pick one of {}",
                other,
                super::FIELDS.join(" | ")
            ));
        }
    };
    edits.color_palette_field = BorderFieldEdit::Set(parsed);
    Ok(())
}

fn stage_corner_or_err(
    slot: &mut BorderFieldEdit<String>,
    label: &str,
    value: &str,
) -> Result<(), String> {
    // Corners pass through the same escape rules as side patterns
    // (so `\(` inside a corner means a literal `(`); we re-use
    // [`SidePattern::parse`] for that and unpack it back into a
    // single concatenated string of clusters. Any parser error
    // surfaces with the corner label.
    let parsed =
        SidePattern::parse(value).map_err(|e| format!("{}: {}", label, e))?;
    let collapsed = match parsed {
        SidePattern::AtomicRepeat { cluster } => cluster.join(""),
        SidePattern::PrefixFillSuffix { .. } => {
            return Err(format!(
                "{}: corner doesn't take a fill region — use a static glyph",
                label
            ));
        }
        // `SidePattern` is `#[non_exhaustive]` so an unrecognised
        // future variant degrades to a clear error rather than a
        // panic — interactive paths must never panic per
        // `CODE_CONVENTIONS.md` §9.
        _ => {
            return Err(format!(
                "{}: unsupported pattern shape for a corner",
                label
            ));
        }
    };
    if collapsed.is_empty() {
        return Err(format!("{}: empty corner glyph", label));
    }
    *slot = BorderFieldEdit::Set(collapsed);
    Ok(())
}

fn parse_pt(key: &str, value: &str) -> Result<f32, String> {
    match value.parse::<f32>() {
        Ok(pt) if pt.is_finite() && pt > 0.0 => Ok(pt),
        Ok(pt) => Err(format!(
            "{}='{}' must be positive and finite; got {}",
            key, value, pt
        )),
        Err(_) => Err(format!("{}='{}' is not a number", key, value)),
    }
}
