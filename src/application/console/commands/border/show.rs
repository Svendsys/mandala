// SPDX-License-Identifier: MPL-2.0

//! `border show` — multi-line readout of the resolved border
//! config for the current selection.
//!
//! Each side row renders in the resolved border font so the user
//! previews the chosen face inline. Operates on the *first*
//! selected node (multi-selection rolls up to one summary; the
//! per-node config may differ but a single readout is cleaner
//! than four-up).

use baumhard::mindmap::border::{
    resolve_border_style, BORDER_APPROX_CHAR_WIDTH_FRAC,
};
use baumhard::mindmap::border_pattern::SidePattern;
use baumhard::mindmap::model::MindNode;

use crate::application::console::parser::Args;
use crate::application::console::{ConsoleEffects, ExecResult, OutputLine};
use crate::application::document::SelectionState;

pub fn execute_border_show(
    _args: &Args,
    eff: &mut ConsoleEffects,
) -> ExecResult {
    let id = match first_selected_node_id(&eff.document.selection) {
        Ok(id) => id,
        Err(msg) => return ExecResult::err(msg),
    };
    let node = match eff.document.mindmap.nodes.get(&id) {
        Some(n) => n,
        None => return ExecResult::err(format!("border: node '{}' not found", id)),
    };
    ExecResult::Lines(format_border_readout(
        node,
        eff.document.mindmap.canvas.default_border.as_ref(),
        &eff.document.mindmap.palettes,
    ))
}

fn first_selected_node_id(sel: &SelectionState) -> Result<String, String> {
    match sel {
        SelectionState::Single(id) => Ok(id.clone()),
        SelectionState::Multi(ids) => ids
            .first()
            .cloned()
            .ok_or_else(|| "border: empty selection".to_string()),
        SelectionState::None => Err("border: no selection".to_string()),
        _ => Err("border: not applicable to this selection".to_string()),
    }
}

fn format_border_readout(
    node: &MindNode,
    canvas_default: Option<&baumhard::mindmap::model::GlyphBorderConfig>,
    palettes: &std::collections::HashMap<
        String,
        baumhard::mindmap::model::Palette,
    >,
) -> Vec<OutputLine> {
    let style = resolve_border_style(
        node.style.border.as_ref(),
        canvas_default,
        &node.style.frame_color,
    );
    let approx_char_width =
        style.font_size_pt * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let char_count = ((node.size.width as f32 / approx_char_width) + 2.0)
        .ceil()
        .max(3.0) as usize;
    let row_count = (node.size.height as f32 / style.font_size_pt)
        .round()
        .max(1.0) as usize;

    let preset_name = node
        .style
        .border
        .as_ref()
        .map(|c| c.preset.as_str())
        .unwrap_or("(default)");
    let visible = node.style.show_frame;

    let palette_summary = match style.color_palette.as_ref() {
        Some(name) => match palettes.get(name) {
            Some(p) => format!(
                "{} (cycling '{}' across {} groups)",
                name,
                style.palette_field.as_str(),
                p.groups.len()
            ),
            None => format!("{} (not found in map)", name),
        },
        None => "(none)".to_string(),
    };

    let face = style.font_name.clone();
    let mut lines: Vec<OutputLine> = Vec::with_capacity(12);
    lines.push(OutputLine::plain(format!(
        "visible: {}",
        if visible { "on" } else { "off" }
    )));
    lines.push(OutputLine::plain(format!("preset:  {}", preset_name)));
    lines.push(OutputLine::plain(format!(
        "font:    {} ({} pt)",
        face.as_deref().unwrap_or("(default)"),
        style.font_size_pt
    )));
    lines.push(OutputLine::plain(format!("color:   {}", style.color)));
    lines.push(OutputLine::plain(format!("palette: {}", palette_summary)));
    // Padding cascades per-node → canvas-default → 4px hardcoded
    // floor. Always print the resolved value so the readout is
    // useful even for nodes with no per-node override (the canvas
    // default may still set a non-default padding).
    let resolved_padding = node
        .style
        .border
        .as_ref()
        .map(|c| c.padding)
        .or_else(|| canvas_default.map(|c| c.padding))
        .unwrap_or(4.0);
    lines.push(OutputLine::plain(format!(
        "padding: {} px",
        resolved_padding
    )));
    lines.push(OutputLine::plain(format!(
        "size:    {}×{} px ({} cluster cols, {} rows)",
        node.size.width as i64,
        node.size.height as i64,
        char_count,
        row_count
    )));

    let side_face = face.clone();
    lines.push(side_line("top:    ", &style.side_patterns.top, char_count, &side_face));
    lines.push(side_line("bottom: ", &style.side_patterns.bottom, char_count, &side_face));
    lines.push(side_line("left:   ", &style.side_patterns.left, row_count, &side_face));
    lines.push(side_line("right:  ", &style.side_patterns.right, row_count, &side_face));
    lines.push(corner_line(&style, &face));
    lines
}

fn side_line(
    label: &str,
    pattern: &SidePattern,
    width: usize,
    face: &Option<String>,
) -> OutputLine {
    let rendered = pattern.render(width).text;
    let text = format!("{}{}", label, rendered);
    match face {
        Some(family) => OutputLine::in_font(text, family),
        None => OutputLine::plain(text),
    }
}

fn corner_line(
    style: &baumhard::mindmap::border::BorderStyle,
    face: &Option<String>,
) -> OutputLine {
    let text = format!(
        "corners: tl={}  tr={}  bl={}  br={}",
        style.corners.top_left,
        style.corners.top_right,
        style.corners.bottom_left,
        style.corners.bottom_right
    );
    match face {
        Some(family) => OutputLine::in_font(text, family),
        None => OutputLine::plain(text),
    }
}
