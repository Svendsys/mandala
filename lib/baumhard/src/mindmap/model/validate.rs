// SPDX-License-Identifier: MPL-2.0

//! Shared validation for the mindmap model's **numeric domain** —
//! `MindNode` / `MindSection` geometry, and every authored number a
//! `.mindmap.json` can carry into the scene build.
//!
//! Kept in the model crate so application setters, the loader, and
//! `maptool verify` reject the same shapes with the same messages.
//!
//! **Why the domain is checked at all.** A `.mindmap.json` is
//! untrusted input (`format/macros.md` §"Threat model"), and the
//! consumers of these numbers are not defensive: cosmic-text
//! asserts that a line height is non-zero, `f32::clamp` panics when
//! its bounds are inverted, and the border and connection builders
//! size a `String` or a `Vec` from authored geometry. A field that
//! is merely *typed* `f32` therefore reaches an `assert!` or an
//! allocator with whatever the file said — including `NaN`,
//! infinity, zero, and 1e30. None of those are recoverable at the
//! point they land: an allocation failure and a stack-exhaustion
//! abort are not panics, so CODE_CONVENTIONS §9's "degrade the
//! frame, log, keep running" has nothing to degrade.
//!
//! So the domain is enforced **once, at the trust boundary**, and
//! the model behind it carries only numbers the renderer can
//! survive. The loader rejects rather than repairs, matching the
//! posture it already takes for structure (zero sections, parent
//! cycles, the section cap): a map that would crash the editor does
//! not open, and the message names the part to fix.

use std::collections::HashMap;

use crate::util::grapheme_chad::count_grapheme_clusters;

use super::{
    Canvas, ControlPoint, EdgeLabelConfig, GlyphBorderConfig, GlyphConnectionConfig, MindEdge, MindMap,
    MindNode, MindSection, PortalEndpointState, Position, Size, MAX_NODE_AXIS,
    MAX_SECTIONS_PER_NODE,
};

/// Floor for any authored font size, in points.
///
/// Zero is the dangerous value — cosmic-text's `Buffer::new`
/// asserts `line_height != 0.0` and aborts the process — but a
/// merely *tiny* size is its own denial of service: the border
/// fill fitter divides a node's width by the glyph advance, so
/// `font_size_pt = 1e-6` asks for hundreds of millions of glyphs in
/// one `String`. Half a point is already invisible at any zoom the
/// camera reaches, so nothing authorable is lost below it.
pub const MIN_FONT_SIZE_PT: f32 = 0.5;

/// Ceiling for any authored font size, in points. Generous enough
/// for a glyph used as a full-screen design element (the canonical
/// fixture's largest run is 74pt) and small enough that a single
/// rasterized glyph cannot exhaust the atlas.
pub const MAX_FONT_SIZE_PT: f32 = 4096.0;

/// Ceiling on the magnitude of an authored canvas coordinate —
/// node positions, section offsets, and Bezier control points.
///
/// Coordinates are signed and nodes float freely, so this bounds
/// `|v|` rather than `v`. The number is deliberately far looser
/// than [`MAX_NODE_AXIS`]: its job is to reject `NaN`, infinity,
/// and values that make a *difference* between two coordinates
/// overflow a sample count, not to have an opinion about how large
/// a canvas may be. The derived counts carry their own caps —
/// see `connection::bezier` and `mindmap::border`.
pub const MAX_CANVAS_COORD: f64 = 1.0e9;

/// Validate a node size candidate, returning the first violation.
pub fn node_size(size: Size) -> Result<(), String> {
    first_or_ok(node_size_violations(size))
}

/// All node-size violations for callers that report rather than
/// reject, such as `maptool verify`.
pub fn node_size_violations(size: Size) -> Vec<String> {
    let mut out = Vec::new();
    if !size.width.is_finite() || !size.height.is_finite() {
        out.push(format!(
            "node.size has non-finite component (width={}, height={})",
            size.width, size.height
        ));
    }
    if size.width.is_finite() && size.width <= 0.0 {
        out.push(format!("node.size.width is not positive ({})", size.width));
    }
    if size.height.is_finite() && size.height <= 0.0 {
        out.push(format!("node.size.height is not positive ({})", size.height));
    }
    if size.width.is_finite() && size.width > MAX_NODE_AXIS {
        out.push(format!(
            "node.size.width ({}) exceeds the {} ceiling; likely a typo (e.g. an extra zero)",
            size.width, MAX_NODE_AXIS
        ));
    }
    if size.height.is_finite() && size.height > MAX_NODE_AXIS {
        out.push(format!(
            "node.size.height ({}) exceeds the {} ceiling; likely a typo (e.g. an extra zero)",
            size.height, MAX_NODE_AXIS
        ));
    }
    out
}

/// Validate the section-count cap for one node.
pub fn section_count(node: &MindNode) -> Result<(), String> {
    if node.sections.len() > MAX_SECTIONS_PER_NODE {
        Err(format!(
            "node.sections.len()={} exceeds cap {}",
            node.sections.len(),
            MAX_SECTIONS_PER_NODE
        ))
    } else {
        Ok(())
    }
}

/// Validate one existing section's AABB against its parent node.
pub fn section_aabb(node: &MindNode, section_idx: usize) -> Result<(), String> {
    let Some(section) = node.sections.get(section_idx) else {
        return Ok(());
    };
    node_size(node.size)?;
    first_or_ok(section_aabb_violations(
        node.size,
        section_idx,
        section.offset,
        section.size,
    ))
}

/// Validate a candidate section AABB against a parent node size.
pub fn section_candidate_aabb(
    parent_size: Size,
    section_idx: usize,
    offset: Position,
    size: Option<Size>,
) -> Result<(), String> {
    node_size(parent_size)?;
    first_or_ok(section_aabb_violations(parent_size, section_idx, offset, size))
}

/// All section-AABB violations for callers that report rather than
/// reject, such as `maptool verify`.
pub fn section_aabb_violations(
    parent_size: Size,
    section_idx: usize,
    offset: Position,
    size: Option<Size>,
) -> Vec<String> {
    let mut out = Vec::new();
    if !offset.x.is_finite() || !offset.y.is_finite() {
        out.push(format!(
            "section[{}].offset has non-finite component (x={}, y={})",
            section_idx, offset.x, offset.y
        ));
    }
    if offset.x.is_finite() && offset.x < 0.0 {
        out.push(format!(
            "section[{}].offset.x is negative ({})",
            section_idx, offset.x
        ));
    }
    if offset.y.is_finite() && offset.y < 0.0 {
        out.push(format!(
            "section[{}].offset.y is negative ({})",
            section_idx, offset.y
        ));
    }
    if let Some(size) = size {
        if !size.width.is_finite() || !size.height.is_finite() {
            out.push(format!(
                "section[{}].size has non-finite component (width={}, height={})",
                section_idx, size.width, size.height
            ));
        }
        if size.width.is_finite() && size.width <= 0.0 {
            out.push(format!(
                "section[{}].size.width is not positive ({})",
                section_idx, size.width
            ));
        }
        if size.height.is_finite() && size.height <= 0.0 {
            out.push(format!(
                "section[{}].size.height is not positive ({})",
                section_idx, size.height
            ));
        }
        if parent_size.width.is_finite() && size.width.is_finite() && size.width > parent_size.width * 100.0 {
            out.push(format!(
                "section[{}].size.width ({}) is over 100× the node's width ({}); \
                 likely a typo (e.g. an extra zero)",
                section_idx, size.width, parent_size.width
            ));
        }
        if parent_size.height.is_finite()
            && size.height.is_finite()
            && size.height > parent_size.height * 100.0
        {
            out.push(format!(
                "section[{}].size.height ({}) is over 100× the node's height ({}); \
                 likely a typo (e.g. an extra zero)",
                section_idx, size.height, parent_size.height
            ));
        }
    }

    let effective = size.unwrap_or(parent_size);
    if offset.x.is_finite()
        && offset.y.is_finite()
        && effective.width.is_finite()
        && effective.height.is_finite()
    {
        let right = offset.x + effective.width;
        let bottom = offset.y + effective.height;
        if right > parent_size.width {
            out.push(format!(
                "section[{}] extends past node right edge ({} > {})",
                section_idx, right, parent_size.width
            ));
        }
        if bottom > parent_size.height {
            out.push(format!(
                "section[{}] extends past node bottom edge ({} > {})",
                section_idx, bottom, parent_size.height
            ));
        }
    }
    out
}

/// Section-channel collision warnings for one node.
pub fn section_channel_collisions(node: &MindNode) -> Vec<String> {
    let mut by_channel: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, section) in node.sections.iter().enumerate() {
        by_channel
            .entry(section.effective_channel(idx))
            .or_default()
            .push(idx);
    }

    let mut out = Vec::new();
    for (channel, indices) in by_channel {
        if indices.len() > 1 {
            out.push(format!(
                "channel {} shared by sections {:?}; mutations targeting that channel \
                 broadcast to all listed sections — usually unintentional",
                channel, indices
            ));
        }
    }
    out
}

fn first_or_ok(messages: Vec<String>) -> Result<(), String> {
    match messages.into_iter().next() {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

// ---- Numeric-domain primitives ------------------------------------

/// Join a label prefix and a field name, tolerating an empty prefix
/// so a top-level field reads `min_zoom_to_render` rather than
/// `.min_zoom_to_render`. The prefix is the *document part* the
/// caller is inside; the loader stamps the node id or edge index
/// on top of whatever comes back.
fn field(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

/// `Some(message)` unless `value` is a font size the text shaper can
/// survive: finite and inside
/// [`MIN_FONT_SIZE_PT`]..=[`MAX_FONT_SIZE_PT`].
fn font_size(label: &str, value: f32) -> Option<String> {
    if !value.is_finite() {
        return Some(format!("{label} is not finite ({value})"));
    }
    if value < MIN_FONT_SIZE_PT {
        return Some(format!(
            "{label} ({value}) is under the {MIN_FONT_SIZE_PT}pt floor — zero aborts the \
             text shaper, and a sub-point size asks the border fitter for millions of glyphs"
        ));
    }
    if value > MAX_FONT_SIZE_PT {
        return Some(format!(
            "{label} ({value}) is over the {MAX_FONT_SIZE_PT}pt ceiling"
        ));
    }
    None
}

/// [`font_size`] for an optional field — absent means "inherit",
/// which is always in domain.
fn opt_font_size(label: &str, value: Option<f32>) -> Option<String> {
    font_size(label, value?)
}

/// `Some(message)` unless `value` is finite and within `±limit`.
fn bounded(label: &str, value: f64, limit: f64) -> Option<String> {
    if !value.is_finite() {
        return Some(format!("{label} is not finite ({value})"));
    }
    if value.abs() > limit {
        return Some(format!("{label} ({value}) is outside the ±{limit} bound"));
    }
    None
}

/// `Some(message)` unless `value` is finite. For fields whose
/// magnitude carries no risk on its own but whose `NaN` poisons a
/// comparison — a zoom bound, a normalized position along a path.
fn finite(label: &str, value: Option<f32>) -> Option<String> {
    let value = value?;
    if value.is_finite() {
        None
    } else {
        Some(format!("{label} is not finite ({value})"))
    }
}

/// `Some(message)` when a min / max pair is inverted.
///
/// This is the [`f32::clamp`] contract, and it is a `panic!`, not a
/// saturation: every size cascade in the edge, label, and portal
/// builders resolves a pair out of the document and clamps with it,
/// so one inverted pair in a file is a crash on the next frame.
fn ordered_pair(label: &str, min: Option<f32>, max: Option<f32>) -> Option<String> {
    let (min, max) = (min?, max?);
    if min > max {
        return Some(format!(
            "{label}: min ({min}) is above max ({max}) — the size clamp panics on an \
             inverted pair"
        ));
    }
    None
}

// ---- Per-struct numeric domain ------------------------------------

/// Every numeric-domain violation on a [`GlyphBorderConfig`],
/// stamped with `label` (the document part that carries it).
pub fn border_config_violations(label: &str, border: &GlyphBorderConfig) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(font_size(&field(label, "font_size_pt"), border.font_size_pt));
    // Padding is a signed inset; only its magnitude and finiteness
    // matter, since the border geometry adds it to an axis already
    // bounded by MAX_NODE_AXIS.
    out.extend(bounded(
        &field(label, "padding"),
        border.padding as f64,
        MAX_NODE_AXIS,
    ));
    out
}

/// Every numeric-domain violation on a [`GlyphConnectionConfig`].
pub fn connection_config_violations(label: &str, conn: &GlyphConnectionConfig) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(font_size(&field(label, "font_size_pt"), conn.font_size_pt));
    out.extend(font_size(
        &field(label, "min_font_size_pt"),
        conn.min_font_size_pt,
    ));
    out.extend(font_size(
        &field(label, "max_font_size_pt"),
        conn.max_font_size_pt,
    ));
    out.extend(ordered_pair(
        label,
        Some(conn.min_font_size_pt),
        Some(conn.max_font_size_pt),
    ));
    // Spacing is added to the glyph advance to derive the sample
    // step; a negative one cancels the advance and a non-finite one
    // poisons the count.
    out.extend(bounded(
        &field(label, "spacing"),
        conn.spacing as f64,
        MAX_NODE_AXIS,
    ));
    if conn.spacing.is_finite() && conn.spacing < 0.0 {
        out.push(format!(
            "{label}.spacing is negative ({}) — a negative gap cancels the glyph \
             advance and the sampler never advances along the path",
            conn.spacing
        ));
    }
    out
}

/// Every numeric-domain violation on an [`EdgeLabelConfig`].
pub fn label_config_violations(label: &str, cfg: &EdgeLabelConfig) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(opt_font_size(&field(label, "font_size_pt"), cfg.font_size_pt));
    out.extend(opt_font_size(
        &field(label, "min_font_size_pt"),
        cfg.min_font_size_pt,
    ));
    out.extend(opt_font_size(
        &field(label, "max_font_size_pt"),
        cfg.max_font_size_pt,
    ));
    out.extend(ordered_pair(label, cfg.min_font_size_pt, cfg.max_font_size_pt));
    out.extend(finite(&field(label, "position_t"), cfg.position_t));
    out.extend(finite(
        &field(label, "perpendicular_offset"),
        cfg.perpendicular_offset,
    ));
    out.extend(zoom_window_violations(
        label,
        cfg.min_zoom_to_render,
        cfg.max_zoom_to_render,
    ));
    out
}

/// Every numeric-domain violation on a [`PortalEndpointState`].
pub fn portal_endpoint_violations(label: &str, portal: &PortalEndpointState) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(opt_font_size(
        &field(label, "text_font_size_pt"),
        portal.text_font_size_pt,
    ));
    out.extend(opt_font_size(
        &field(label, "text_min_font_size_pt"),
        portal.text_min_font_size_pt,
    ));
    out.extend(opt_font_size(
        &field(label, "text_max_font_size_pt"),
        portal.text_max_font_size_pt,
    ));
    out.extend(ordered_pair(
        &field(label, "text"),
        portal.text_min_font_size_pt,
        portal.text_max_font_size_pt,
    ));
    out.extend(finite(&field(label, "border_t"), portal.border_t));
    out.extend(finite(
        &field(label, "perpendicular_offset"),
        portal.perpendicular_offset,
    ));
    out.extend(zoom_window_violations(
        label,
        portal.min_zoom_to_render,
        portal.max_zoom_to_render,
    ));
    out
}

/// Violations on a zoom window — the `min` / `max` pair every
/// zoom-gated part of the format carries.
///
/// A `NaN` bound is the interesting case: every comparison against
/// it is false, so the element silently never renders and the
/// author has no way to see why. Both bounds are inclusive and a
/// zoom is a positive scale, so a negative bound is authored
/// nonsense rather than a wider window.
pub fn zoom_window_violations(label: &str, min: Option<f32>, max: Option<f32>) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(finite(&field(label, "min_zoom_to_render"), min));
    out.extend(finite(&field(label, "max_zoom_to_render"), max));
    for (field, value) in [("min_zoom_to_render", min), ("max_zoom_to_render", max)] {
        if let Some(value) = value {
            if value.is_finite() && value < 0.0 {
                out.push(format!("{label}.{field} is negative ({value})"));
            }
        }
    }
    out.extend(ordered_pair(&field(label, "zoom"), min, max));
    out
}

/// Every numeric-domain violation across one section's `text_runs`.
///
/// Two separate concerns. Each run's `size_pt` is the font size the
/// shaper receives, so it lives under the same floor and ceiling as
/// every other size. The run *ranges* are the second: the model
/// documents them as sorted, half-open `[start, end)`, non-
/// overlapping grapheme ranges, and `text_run_ops` relies on that
/// — but nothing enforced it on the way in. Overlapping runs make
/// the renderer's span builder emit the same graphemes once per
/// covering run, so N nested runs over one string is a quadratic
/// blow-up in shaped text from a linear file.
fn text_run_violations(section_idx: usize, section: &MindSection) -> Vec<String> {
    let mut out = Vec::new();
    if section.text_runs.is_empty() {
        return out;
    }
    // Grapheme clusters, not bytes — `start` / `end` index what the
    // user sees (`format/text-runs.md`), which is also the unit
    // `maptool verify` counts in.
    let total = count_grapheme_clusters(&section.text);
    let mut previous_end: Option<usize> = None;

    for (run_idx, run) in section.text_runs.iter().enumerate() {
        let label = format!("section[{section_idx}].text_runs[{run_idx}]");
        if run.size_pt == 0 {
            out.push(format!(
                "{label}.size_pt is zero — the text shaper asserts on a zero line height"
            ));
        } else if (run.size_pt as f32) > MAX_FONT_SIZE_PT {
            out.push(format!(
                "{label}.size_pt ({}) is over the {MAX_FONT_SIZE_PT}pt ceiling",
                run.size_pt
            ));
        }
        // `>=`, not `>`: a zero-length run is as malformed as an
        // inverted one. `text_run_ops` calls the pair "zero-length
        // or inverted" and the document layer carries a
        // `debug_assert` that a run is non-empty, so accepting
        // `start == end` here leaves an abort reachable in every
        // debug build — including `cargo run`, the documented
        // one-off iteration command.
        if run.start >= run.end {
            out.push(format!(
                "{label} has start {} not less than end {} — a zero-length or inverted run",
                run.start, run.end
            ));
            // Its range is meaningless, so it neither bounds the
            // next run nor gets a length check. Mirrors verify.
            continue;
        }
        if run.end > total {
            out.push(format!(
                "{label} end {} exceeds text length {total} (grapheme clusters)",
                run.end
            ));
        }
        if let Some(previous_end) = previous_end {
            if run.start < previous_end {
                out.push(format!(
                    "{label} overlaps previous run (start {} < previous end {previous_end}) — \
                     overlapping runs re-emit the same graphemes once per covering run",
                    run.start
                ));
            }
        }
        previous_end = Some(run.end);
    }
    out
}

/// Every numeric-domain violation carried by one section.
///
/// Deliberately **not** the AABB containment rules: a section that
/// hangs past its node's edge renders fine and is
/// `maptool verify`'s call (`format/schema.md`). What belongs here
/// is only what the renderer cannot survive — a non-finite or
/// degenerate extent, a hostile border config, a bad run.
pub fn section_numeric_violations(idx: usize, section: &MindSection) -> Vec<String> {
    let mut out = Vec::new();
    let label = format!("section[{idx}]");
    out.extend(bounded(&field(&label, "offset.x"), section.offset.x, MAX_CANVAS_COORD));
    out.extend(bounded(&field(&label, "offset.y"), section.offset.y, MAX_CANVAS_COORD));
    if let Some(size) = section.size {
        out.extend(bounded(&field(&label, "size.width"), size.width, MAX_NODE_AXIS));
        out.extend(bounded(&field(&label, "size.height"), size.height, MAX_NODE_AXIS));
        if size.width.is_finite() && size.width <= 0.0 {
            out.push(format!("{label}.size.width is not positive ({})", size.width));
        }
        if size.height.is_finite() && size.height <= 0.0 {
            out.push(format!("{label}.size.height is not positive ({})", size.height));
        }
    }
    if let Some(border) = section.frame_border.as_ref() {
        out.extend(border_config_violations(&field(&label, "frame_border"), border));
    }
    out.extend(text_run_violations(idx, section));
    out
}

/// Every numeric-domain violation carried by one [`MindNode`] —
/// its own geometry, its style and layout numbers, its border
/// override, and each of its sections.
///
/// Cost: O(sections + text runs) on the node; no allocation beyond
/// the returned messages, which are empty for a well-formed node.
pub fn node_numeric_violations(node: &MindNode) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(bounded("position.x", node.position.x, MAX_CANVAS_COORD));
    out.extend(bounded("position.y", node.position.y, MAX_CANVAS_COORD));
    out.extend(node_size_violations(node.size));
    out.extend(bounded(
        "style.corner_radius_percent",
        node.style.corner_radius_percent,
        MAX_NODE_AXIS,
    ));
    out.extend(bounded(
        "style.frame_thickness",
        node.style.frame_thickness,
        MAX_NODE_AXIS,
    ));
    out.extend(bounded("layout.spacing", node.layout.spacing, MAX_CANVAS_COORD));
    if let Some(border) = node.style.border.as_ref() {
        out.extend(border_config_violations("style.border", border));
    }
    out.extend(zoom_window_violations(
        "",
        node.min_zoom_to_render,
        node.max_zoom_to_render,
    ));
    for (idx, section) in node.sections.iter().enumerate() {
        out.extend(section_numeric_violations(idx, section));
    }
    out
}

/// Every numeric-domain violation carried by one [`MindEdge`] —
/// its control points, its glyph-connection override, its label
/// config, and both portal endpoints.
pub fn edge_numeric_violations(edge: &MindEdge) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, point) in edge.control_points.iter().enumerate() {
        out.extend(control_point_violations(idx, point));
    }
    if let Some(conn) = edge.glyph_connection.as_ref() {
        out.extend(connection_config_violations("glyph_connection", conn));
    }
    if let Some(cfg) = edge.label_config.as_ref() {
        out.extend(label_config_violations("label_config", cfg));
    }
    if let Some(portal) = edge.portal_from.as_ref() {
        out.extend(portal_endpoint_violations("portal_from", portal));
    }
    if let Some(portal) = edge.portal_to.as_ref() {
        out.extend(portal_endpoint_violations("portal_to", portal));
    }
    out.extend(zoom_window_violations(
        "",
        edge.min_zoom_to_render,
        edge.max_zoom_to_render,
    ));
    out
}

/// Violations on one Bezier control point. Control-point offsets
/// feed the path sampler, whose sample count is the path length
/// over the glyph step — so a non-finite or astronomical offset is
/// what turns one edge into an unbounded allocation.
fn control_point_violations(idx: usize, point: &ControlPoint) -> Vec<String> {
    let label = format!("control_points[{idx}]");
    let mut out = Vec::new();
    out.extend(bounded(&field(&label, "x"), point.x, MAX_CANVAS_COORD));
    out.extend(bounded(&field(&label, "y"), point.y, MAX_CANVAS_COORD));
    out
}

/// Every numeric-domain violation carried by the [`Canvas`]'s
/// default border and connection cascades.
///
/// These matter out of proportion to their size: a canvas default
/// is the fallback for *every* node or edge that does not override
/// it, so one hostile number here poisons the whole document at
/// once rather than a single element.
pub fn canvas_numeric_violations(canvas: &Canvas) -> Vec<String> {
    let mut out = Vec::new();
    for (label, border) in [
        ("default_border", canvas.default_border.as_ref()),
        ("default_section_frame_border", canvas.default_section_frame_border.as_ref()),
        (
            "default_focused_section_frame_border",
            canvas.default_focused_section_frame_border.as_ref(),
        ),
    ] {
        if let Some(border) = border {
            out.extend(border_config_violations(label, border));
        }
    }
    if let Some(conn) = canvas.default_connection.as_ref() {
        out.extend(connection_config_violations("default_connection", conn));
    }
    out
}

/// The whole-map numeric-domain sweep the loader runs, returning
/// the first violation stamped with the document part that carries
/// it — `canvas`, `node "1.2"`, `edge[3]` — matching how the
/// loader's other rejections address the file.
///
/// Nodes are visited in sorted-id order so the reported part is
/// deterministic across `HashMap` iteration order, the same reason
/// the zero-section and section-cap checks sort.
///
/// Cost: O(nodes + sections + runs + edges + control points), one
/// pass, allocating only on the failure it reports.
pub fn map_numeric_domain(map: &MindMap) -> Option<String> {
    if let Some(message) = canvas_numeric_violations(&map.canvas).into_iter().next() {
        return Some(format!("canvas: {message}"));
    }
    let mut nodes: Vec<&MindNode> = map.nodes.values().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    for node in nodes {
        if let Some(message) = node_numeric_violations(node).into_iter().next() {
            return Some(format!("node {:?}: {message}", node.id));
        }
    }
    for (idx, edge) in map.edges.iter().enumerate() {
        if let Some(message) = edge_numeric_violations(edge).into_iter().next() {
            return Some(format!("edge[{idx}]: {message}"));
        }
    }
    None
}
