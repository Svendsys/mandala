// SPDX-License-Identifier: MPL-2.0

//! `color bg=#009c15 text=accent border=reset` — kv-form color
//! setter dispatched through the capability traits. Each key maps to
//! a trait (`bg` → HasBgColor, `text` → HasTextColor, `border` →
//! HasBorderColor). Fans out over the selection; reports per-pair
//! outcome so a pair that's not applicable to one target doesn't
//! sink the whole command.
//!
//! Axis-only positionals (`color bg`, `color text`, `color border`)
//! and the legacy `color pick` both hand off to the glyph-wheel
//! picker modal — `color bg` picks a color for that axis on the
//! current selection.

use super::Command;
use crate::application::color_picker::{ColorTarget, NodeColorAxis, SectionColorAxis};
use crate::application::console::completion::{
    kv_key_completions_with_hints, prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::traits::{
    apply_kvs, ColorValue, HasBgColor, HasBorderColor, HasTextColor, Outcome,
};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{SectionSel, SelectionState};

pub const KEYS: &[&str] = &["bg", "text", "border", "section"];
pub const VALUE_PRESETS: &[&str] = &["accent", "edge", "fg", "reset"];

pub const COMMAND: Command = Command {
    name: "color",
    aliases: &[],
    summary: "Set bg/text/border color, or pick via the glyph wheel",
    usage: "color bg=<color> text=<color> border=<color>   |   color bg|text|border|pick",
    tags: &["color", "bg", "text", "border", "pick", "wheel"],
    applicable: always,
    complete: complete_color,
    execute: execute_color,
};

fn complete_color(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index } => {
            let mut out = kv_key_completions_with_hints(KEYS, state.partial, kv_hint);
            // At token 0 the bare verbs — `pick` plus the axis
            // positionals `bg` / `text` / `border` — also hand off
            // to the glyph-wheel picker. Suggest them alongside the
            // kv-key forms.
            if *index == 0 {
                out.extend(prefix_filter(&["pick", "picker"], state.partial));
            }
            // `color picker` expects `on` / `off` as the next token.
            if *index == 1 && matches!(state.tokens.first().map(String::as_str), Some("picker")) {
                out.extend(prefix_filter(&["on", "off"], state.partial));
            }
            out
        }
        CompletionContext::KvValue { key } if KEYS.iter().any(|k| k == key) => {
            prefix_filter(VALUE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn kv_hint(key: &str) -> Option<&'static str> {
    match key {
        "bg" => Some("fill / background color"),
        "text" => Some("text / label color"),
        "border" => Some("frame / line color"),
        "section" => Some("target section index inside a multi-section node"),
        _ => None,
    }
}

/// Outcome of resolving a positional `color` verb against the
/// current selection. Distinguishes "open the picker on this
/// target" from "the verb doesn't apply to this selection shape"
/// (descriptive message) from "no selection / unknown verb" (fall
/// through to the generic error). Lets `bg`/`border` on a section
/// surface a clearer reason than the generic fallback.
enum PickerTargetOutcome {
    Open(ColorTarget),
    NotApplicable(String),
    Unknown,
}

/// Map a bare positional verb (`pick`, `bg`, `text`, `border`) to a
/// concrete `ColorTarget` based on the current selection.
///
/// Node targets carry the axis directly. Edge / portal targets
/// collapse axis into their one color field: `bg`/`border` on an
/// edge both resolve to the edge's line color; `bg` on a portal
/// resolves to the portal's fill. Section targets honour the `text`
/// axis and report NotApplicable for `bg` / `border` (sections have
/// no chrome by spec — see `format/sections.md`).
fn picker_target_for(verb: &str, selection: &SelectionState) -> PickerTargetOutcome {
    let axis = match verb {
        "bg" => Some(NodeColorAxis::Bg),
        "text" => Some(NodeColorAxis::Text),
        "border" => Some(NodeColorAxis::Border),
        "pick" => None, // axis-agnostic legacy flow
        _ => return PickerTargetOutcome::Unknown,
    };
    match selection {
        SelectionState::Single(id) => match axis {
            Some(a) => PickerTargetOutcome::Open(ColorTarget::Node {
                id: id.clone(),
                axis: a,
            }),
            // `color pick` on a node defaults to bg.
            None => PickerTargetOutcome::Open(ColorTarget::Node {
                id: id.clone(),
                axis: NodeColorAxis::Bg,
            }),
        },
        // Section selection: route the picker to the targeted
        // section so commit lands on `set_section_text_color`,
        // leaving sibling sections untouched. `bg`/`border` have
        // no section-level fields (matches the kv-form
        // `apply_section_colours` arm below) — surface a clear
        // NotApplicable message rather than collapsing to the
        // owning node, which would silently broaden the user's
        // intent.
        SelectionState::Section(SectionSel { node_id, section_idx }) => match axis {
            Some(NodeColorAxis::Text) | None => PickerTargetOutcome::Open(ColorTarget::Section {
                node_id: node_id.clone(),
                section_idx: *section_idx,
                axis: SectionColorAxis::Text,
            }),
            Some(NodeColorAxis::Bg) => PickerTargetOutcome::NotApplicable(
                "color bg: not applicable to a section (section-level chrome doesn't exist)".to_string(),
            ),
            Some(NodeColorAxis::Border) => PickerTargetOutcome::NotApplicable(
                "color border: not applicable to a section (section-level chrome doesn't exist)"
                    .to_string(),
            ),
        },
        SelectionState::Multi(ids) => {
            // The picker is single-target; pick the first node in
            // the multi-selection. Fanout through the picker is
            // a future addition.
            match ids.first() {
                Some(id) => PickerTargetOutcome::Open(ColorTarget::Node {
                    id: id.clone(),
                    axis: axis.unwrap_or(NodeColorAxis::Bg),
                }),
                None => PickerTargetOutcome::Unknown,
            }
        }
        SelectionState::Edge(er) => {
            // Edges (line-mode or portal-mode) have one color
            // field. `border` maps to it, `text` also currently
            // maps to it (edge label + line share `MindEdge.color`),
            // and for portal-mode edges `bg` is accepted as an
            // alias because "fill" reads more natural there.
            PickerTargetOutcome::Open(ColorTarget::Edge(er.clone()))
        }
        SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
            // Portal icon or portal text — both share the same
            // owning edge identity. The axis is irrelevant at the
            // picker level (one color field per endpoint channel);
            // the commit path reads the active selection variant
            // to decide whether to write `color` (icon) or
            // `text_color`. Returning the owning-edge target here
            // keeps the picker target resolution shape identical
            // to the `Edge` branch; per-variant routing lives in
            // the commit path.
            PickerTargetOutcome::Open(ColorTarget::Edge(s.edge_ref()))
        }
        SelectionState::EdgeLabel(s) => {
            // Line-mode label: same owning-edge shape as `Edge`;
            // the commit path discriminates between edge-body and
            // label color writes via the active selection variant.
            PickerTargetOutcome::Open(ColorTarget::Edge(s.edge_ref.clone()))
        }
        SelectionState::None => PickerTargetOutcome::Unknown,
    }
}

fn execute_color(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Positional handoffs to the glyph-wheel picker:
    //  - `color pick` — legacy edge/portal one-axis flow
    //  - `color bg | text | border` — pick a color for that axis on
    //    the current selection (node axis for nodes, single-color
    //    target for edges/portals)
    //  - `color picker on` — open the picker as a persistent
    //    standalone palette (no target; commit applies to selection)
    //  - `color picker off` — close any open picker
    if let Some(verb) = args.positional(0) {
        if verb == "picker" {
            match args.positional(1) {
                Some("on") => {
                    eff.open_color_picker_standalone = true;
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                Some("off") => {
                    eff.close_color_picker = true;
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                _ => return ExecResult::err("usage: color picker on | color picker off"),
            }
        }
        match picker_target_for(verb, &eff.document.selection) {
            PickerTargetOutcome::Open(target) => {
                eff.open_color_picker = Some(target);
                eff.close_console = true;
                return ExecResult::ok_empty();
            }
            PickerTargetOutcome::NotApplicable(msg) => {
                return ExecResult::err(msg);
            }
            PickerTargetOutcome::Unknown => {}
        }
        if matches!(verb, "pick" | "bg" | "text" | "border") {
            return ExecResult::err(format!("color {}: nothing to pick for this selection", verb));
        }
    }

    // Split out the optional `section=N` from the colour kvs. When
    // present, the verb routes per-section through
    // `set_section_text_color` rather than the whole-node trait
    // dispatcher — that's the only setter today that accepts a
    // section index. `bg` / `border` aren't section-level fields,
    // so an explicit `section=N` paired with them surfaces a
    // NotApplicable error rather than silently ignoring the index.
    let mut section_target: Option<usize> = None;
    let mut colour_kvs: Vec<(String, String)> = Vec::new();
    for (k, v) in args.kvs() {
        if k == "section" {
            match v.parse::<usize>() {
                Ok(idx) => section_target = Some(idx),
                Err(_) => {
                    return ExecResult::err(format!(
                        "color: section='{}' is not a non-negative integer",
                        v
                    ));
                }
            }
        } else {
            colour_kvs.push((k.to_string(), v.to_string()));
        }
    }
    if colour_kvs.is_empty() && section_target.is_none() {
        return ExecResult::err("usage: color bg|text|border[=<color>]   |   color pick");
    }
    if colour_kvs.is_empty() {
        return ExecResult::err("color: section=N requires at least one colour axis (e.g. text=#ff0000)");
    }

    if let Some(idx) = section_target {
        return apply_section_colours(eff.document, idx, &colour_kvs);
    }

    let report = apply_kvs(eff.document, &colour_kvs, |view, key, value| {
        let color = match ColorValue::parse(value) {
            Ok(c) => c,
            Err(msg) => return Some(Outcome::Invalid(msg)),
        };
        match key {
            "bg" => Some(view.set_bg_color(color)),
            "text" => Some(view.set_text_color(color)),
            "border" => Some(view.set_border_color(color)),
            _ => None,
        }
    });

    finalize_report(report, "color")
}

/// Per-section colour write. `text` routes through
/// [`super::super::super::document::MindMapDocument::set_section_text_color`];
/// `bg` / `border` aren't section-level fields and surface a
/// NotApplicable message rather than landing on the whole-node
/// chrome (that would surprise authors who deliberately scoped
/// to one section).
fn apply_section_colours(
    doc: &mut crate::application::document::MindMapDocument,
    section_idx: usize,
    kvs: &[(String, String)],
) -> ExecResult {
    let node_id = match doc.selection.clone() {
        SelectionState::Single(id) => id,
        SelectionState::Section(SectionSel { node_id, .. }) => node_id,
        _ => return ExecResult::err("color: section=N requires a node or section selection"),
    };
    let mut messages = Vec::new();
    let mut any_applied = false;
    for (k, v) in kvs {
        match k.as_str() {
            "text" => {
                let color_value = match ColorValue::parse(v) {
                    Ok(c) => c,
                    Err(msg) => {
                        messages.push(format!("text: {}", msg));
                        continue;
                    }
                };
                let resolved = match color_value {
                    ColorValue::Hex(h) => h,
                    ColorValue::Var(name) => format!("var(--{})", name),
                    ColorValue::Reset => "#ffffff".to_string(),
                };
                if doc.set_section_text_color(&node_id, section_idx, resolved) {
                    any_applied = true;
                }
            }
            "bg" | "border" => {
                messages.push(format!(
                    "{}: not applicable to a section (section-level chrome doesn't exist)",
                    k
                ));
            }
            other => messages.push(format!("unknown key '{}'", other)),
        }
    }
    if any_applied && messages.is_empty() {
        return ExecResult::ok_msg(format!("color applied to section {}", section_idx));
    }
    if any_applied {
        return ExecResult::lines(messages);
    }
    if messages.is_empty() {
        return ExecResult::ok_msg("color: no change");
    }
    ExecResult::err(messages.join("; "))
}

/// Mutation core: apply a single color axis (`bg|text|border`) to
/// the current selection. Both the kv-form `color` console verb
/// (which dispatches multiple kvs at once via `apply_kvs`) and the
/// parametric `Action::SetColor*` Action arms route through the
/// same trait dispatch — this helper is the single-kv wrapper.
///
/// Returns `true` when at least one target actually changed; `false`
/// otherwise (no selection, invalid color string, every target was
/// already at the requested color, or the axis isn't applicable to
/// the selection kind). The Action arm uses the bool to decide
/// whether to trigger a scene rebuild; the verb keeps its full
/// per-pair outcome reporting.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_color_axis_to_selection(
    doc: &mut crate::application::document::MindMapDocument,
    axis: &str,
    value: &str,
) -> bool {
    let kvs = vec![(axis.to_string(), value.to_string())];
    let report = apply_kvs(doc, &kvs, |view, key, value| {
        let color = match ColorValue::parse(value) {
            Ok(c) => c,
            Err(msg) => return Some(Outcome::Invalid(msg)),
        };
        match key {
            "bg" => Some(view.set_bg_color(color)),
            "text" => Some(view.set_text_color(color)),
            "border" => Some(view.set_border_color(color)),
            _ => None,
        }
    });
    report.any_applied
}

/// Common report-to-ExecResult conversion used by every
/// trait-dispatched command.
pub(super) fn finalize_report(
    report: crate::application::console::traits::DispatchReport,
    verb: &str,
) -> ExecResult {
    if report.all_failed {
        return ExecResult::err(report.messages.join("; "));
    }
    if !report.messages.is_empty() {
        return ExecResult::lines(report.messages);
    }
    if report.any_applied {
        ExecResult::ok_msg(format!("{} applied", verb))
    } else {
        ExecResult::ok_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::{parse, ParseResult};
    use crate::application::document::tests_common::{first_testament_node_id, load_test_doc};

    #[test]
    fn apply_color_axis_writes_bg_to_node() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        let changed = apply_color_axis_to_selection(&mut doc, "bg", "#fafafa");
        assert!(changed);
        let style = &doc.mindmap.nodes.get(&id).unwrap().style;
        assert_eq!(style.background_color, "#fafafa");
    }

    #[test]
    fn apply_color_axis_returns_false_with_no_selection() {
        let mut doc = load_test_doc();
        // Default selection is None — nothing to target.
        assert!(!apply_color_axis_to_selection(&mut doc, "bg", "#fafafa"));
    }

    #[test]
    fn apply_color_axis_returns_false_for_invalid_color() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        // ColorValue::parse rejects this; the trait dispatcher
        // reports `Invalid` per target. `any_applied` stays false.
        assert!(!apply_color_axis_to_selection(
            &mut doc,
            "bg",
            "definitely-not-a-color"
        ));
    }

    #[test]
    fn apply_color_axis_returns_false_for_unknown_axis() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        // The closure returns None for unknown keys; the dispatcher
        // surfaces "unknown key" as a message and `any_applied`
        // stays false.
        assert!(!apply_color_axis_to_selection(&mut doc, "bogus_axis", "#fafafa"));
    }

    /// `color text=#... section=K` routes through
    /// `set_section_text_color` for the specified index — runs on
    /// the targeted section get the new colour, runs on other
    /// sections stay untouched.
    #[test]
    fn color_text_section_kv_targets_specific_section() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};
        use baumhard::mindmap::model::MindSection;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections
                .push(MindSection::new_default("second".into(), Vec::new()));
            // `set_section_text_color` writes only over runs whose
            // colour matches the node's `style.text_color` default —
            // pin the node default to `#aaaaaa` so the test's
            // synthetic runs match the predicate.
            node.style.text_color = "#aaaaaa".into();
            for section in node.sections.iter_mut() {
                section.text_runs.clear();
                section.text_runs.push(baumhard::mindmap::model::TextRun {
                    start: 0,
                    end: section.text.chars().count().max(1),
                    bold: false,
                    italic: false,
                    underline: false,
                    font: "LiberationSans".into(),
                    size_pt: 14,
                    color: "#aaaaaa".into(),
                    hyperlink: None,
                });
            }
        }
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("color text=#ff0000 section=1", &mut doc));
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.color == "#aaaaaa"),
            "section 0 must NOT receive the colour change"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.color == "#ff0000"),
            "section 1 must receive the new colour"
        );
    }

    /// Build a node with two sections (each with one run pinned to
    /// `#aaaaaa`, matching the node's `style.text_color` default so
    /// the cascade predicate inside `set_section_text_color` finds
    /// runs to rewrite). Returns `(doc, node_id)`. Mirrors the
    /// scaffolding in `color_text_section_kv_targets_specific_section`.
    fn doc_with_two_sections() -> (crate::application::document::MindMapDocument, String) {
        use baumhard::mindmap::model::MindSection;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections
                .push(MindSection::new_default("second".into(), Vec::new()));
            node.style.text_color = "#aaaaaa".into();
            for section in node.sections.iter_mut() {
                section.text_runs.clear();
                section.text_runs.push(baumhard::mindmap::model::TextRun {
                    start: 0,
                    end: section.text.chars().count().max(1),
                    bold: false,
                    italic: false,
                    underline: false,
                    font: "LiberationSans".into(),
                    size_pt: 14,
                    color: "#aaaaaa".into(),
                    hyperlink: None,
                });
            }
        }
        (doc, id)
    }

    /// `color text=#…` with a `SelectionState::Section` (no explicit
    /// `section=K` kv) routes through the `HasTextColor` trait arm
    /// to `set_section_text_color` — only the targeted section's
    /// runs change, siblings stay untouched. Pre-Tier-2A this
    /// collapsed to whole-node and silently broadened the user's
    /// intent. Pins Item 1 of `SECTION_INTEGRATION_PLAN.md`.
    #[test]
    fn color_text_section_collapse_writes_only_section() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("color text=#00ff00", &mut doc));
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.color == "#aaaaaa"),
            "section 0 (sibling) must NOT receive the colour change"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.color == "#00ff00"),
            "section 1 (selected) must receive the new colour"
        );
    }

    /// `color bg=#…` with a `SelectionState::Section` reports
    /// NotApplicable rather than collapsing to the owning node's
    /// `background_color`. Sections have no bg-fill chrome by spec
    /// (`format/sections.md`). Pins Item 2.
    #[test]
    fn color_bg_section_returns_not_applicable() {
        use crate::application::console::tests::fixtures::{join_lines, run};
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        let original_bg = doc.mindmap.nodes.get(&id).unwrap().style.background_color.clone();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        match run("color bg=#123456", &mut doc) {
            ExecResult::Lines(msgs) => assert!(
                join_lines(&msgs).contains("not applicable"),
                "expected NotApplicable surface; got {:?}",
                msgs
            ),
            ExecResult::Err(s) => assert!(s.contains("not applicable"), "got Err({:?})", s),
            other => panic!("expected Lines / Err with 'not applicable', got {:?}", other),
        }
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().style.background_color,
            original_bg,
            "node bg must NOT change when bg= targets a section selection"
        );
    }

    /// Mirror of `color_bg_section_returns_not_applicable` for the
    /// `border` axis — sections have no frame chrome either. Pins
    /// Item 3.
    #[test]
    fn color_border_section_returns_not_applicable() {
        use crate::application::console::tests::fixtures::{join_lines, run};
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        let original_frame = doc.mindmap.nodes.get(&id).unwrap().style.frame_color.clone();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        match run("color border=#abcdef", &mut doc) {
            ExecResult::Lines(msgs) => assert!(
                join_lines(&msgs).contains("not applicable"),
                "expected NotApplicable surface; got {:?}",
                msgs
            ),
            ExecResult::Err(s) => assert!(s.contains("not applicable"), "got Err({:?})", s),
            other => panic!("expected Lines / Err with 'not applicable', got {:?}", other),
        }
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().style.frame_color,
            original_frame,
            "node frame must NOT change when border= targets a section selection"
        );
    }

    /// `color text` (no value) on a `SelectionState::Section` opens
    /// the picker bound to a `ColorTarget::Section` — the picker's
    /// commit then writes through `set_section_text_color`. Pins
    /// Item 7 (text-axis branch).
    #[test]
    fn picker_target_for_section_text_emits_section_target() {
        use crate::application::color_picker::{ColorTarget, SectionColorAxis};
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let (cmd, toks) = match parse("color text") {
            ParseResult::Ok { cmd, args } => (cmd, args),
            _ => panic!("parse failed"),
        };
        let mut eff = ConsoleEffects::new(&mut doc);
        let _ = (cmd.execute)(&Args::new(&toks), &mut eff);
        match eff.open_color_picker {
            Some(ColorTarget::Section {
                node_id,
                section_idx,
                axis,
            }) => {
                assert_eq!(node_id, id);
                assert_eq!(section_idx, 1);
                assert_eq!(axis, SectionColorAxis::Text);
            }
            other => panic!("expected ColorTarget::Section/Text, got {:?}", other),
        }
    }

    /// `color bg` on a `SelectionState::Section` returns
    /// NotApplicable with a descriptive message (no picker opens,
    /// no silent collapse to the owning node's bg axis). Pins Item
    /// 7 (bg/border-axis branch).
    #[test]
    fn picker_target_for_section_bg_returns_not_applicable_message() {
        use crate::application::console::tests::fixtures::assert_exec_err_contains;
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let (cmd, toks) = match parse("color bg") {
            ParseResult::Ok { cmd, args } => (cmd, args),
            _ => panic!("parse failed"),
        };
        let mut eff = ConsoleEffects::new(&mut doc);
        let result = (cmd.execute)(&Args::new(&toks), &mut eff);
        assert!(
            eff.open_color_picker.is_none(),
            "no picker should open for bg axis on a section selection"
        );
        assert_exec_err_contains(result, "not applicable");
    }
}
