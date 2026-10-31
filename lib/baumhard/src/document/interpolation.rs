// SPDX-License-Identifier: MPL-2.0

//! The document's state between a journal's before and after: what
//! moves smoothly and what switches.
//!
//! **The fields #237 lists interpolate; everything else snaps.** A
//! node's position and minimum size, a section's offset and minimum
//! size, an opacity, a fill fade, a font size of one unit, a literal
//! color (a palette group's included), a frame's gap and inset, a path's
//! spacing and size, its control points when both sides have as many, a
//! label's `t` and offset, a portal end's `border_t`, offset and size,
//! and the theme's variant values — each moves along the straight line
//! from before to after. Everything else switches whole at the snap
//! point: text, fonts, weights, flags, ids, links, a color given by a
//! theme variable or a palette, and every enum, with what it carries —
//! a shape and its corner radius, a fit mode and its range, a frame's
//! start point — and a zoom window, and a frame's or ring's `span`,
//! `sides` and `spacing` (#291), which #237's list predates. A field interpolates only when both
//! sides hold a value of the same form; otherwise it snaps, so a field
//! that appears or disappears switches rather than inventing a value.
//! (B1 blended fit ranges, corner radii, zoom windows and start points
//! too; ruling R10 on #237 holds them to the list.)
//!
//! Every interpolated element starts as a clone of the side the snap
//! picks ([`Snap::pick`](crate::engine::interpolate::Snap::pick)), and
//! only the interpolating fields are overwritten. The **key** — a
//! node's, section's or ring's channel, a portal end's `end` — is
//! never one of them, so it always comes from the same side as the
//! element's place in its row, which keeps every row sorted at every
//! `t` (the `Interpolable` contract). At `t = 0`
//! each lerp returns its `from` exactly and at `t = 1` its `to`, so the
//! endpoints hold the before and after states exactly.

use crate::document::element::{
    DefaultsData, DocElement, FlowStyle, FrameStyle, LabelStyle, NodeStyle, PaletteGroup, PathData,
    PortalEndStyle, RingStyle, TextStyle, ThemeData,
};
use crate::document::values::{ColorRef, FontSize, PlainColor, Rgba};
use crate::engine::interpolate::{Interpolable, Interpolate, Snap};
use glam::Vec2;

/// Per channel, rounded to the nearest byte (the engine's `[u8; 4]`
/// blend).
impl Interpolate for Rgba {
    fn lerp(from: &Rgba, to: &Rgba, t: f32) -> Rgba {
        Rgba(<[u8; 4]>::lerp(&from.0, &to.0, t))
    }
}

/// `value` blended between `from` and `to` when both are set; left as
/// the snapped side holds it otherwise.
fn opt<T: Interpolate>(value: &mut Option<T>, from: &Option<T>, to: &Option<T>, t: f32) {
    if let (Some(from), Some(to)) = (from, to) {
        *value = Some(T::lerp(from, to, t));
    }
}

/// A literal color blends; a variable or a palette field snaps.
fn color_ref(value: &mut Option<ColorRef>, from: &Option<ColorRef>, to: &Option<ColorRef>, t: f32) {
    if let (Some(ColorRef::Rgba(from)), Some(ColorRef::Rgba(to))) = (from, to) {
        *value = Some(ColorRef::Rgba(Rgba::lerp(from, to, t)));
    }
}

/// A palette color: a literal blends; a variable snaps.
fn plain(value: &mut PlainColor, from: &PlainColor, to: &PlainColor, t: f32) {
    if let (PlainColor::Rgba(from), PlainColor::Rgba(to)) = (from, to) {
        *value = PlainColor::Rgba(Rgba::lerp(from, to, t));
    }
}

/// A font size of one unit blends; across units it snaps.
fn font_size(value: &mut Option<FontSize>, from: &Option<FontSize>, to: &Option<FontSize>, t: f32) {
    match (from, to) {
        (Some(FontSize::Pt(a)), Some(FontSize::Pt(b))) => {
            *value = Some(FontSize::Pt(<f32 as Interpolate>::lerp(a, b, t)))
        }
        (Some(FontSize::Rel(a)), Some(FontSize::Rel(b))) => {
            *value = Some(FontSize::Rel(<f32 as Interpolate>::lerp(a, b, t)))
        }
        _ => {}
    }
}

fn text(value: &mut TextStyle, from: &TextStyle, to: &TextStyle, t: f32) {
    font_size(&mut value.size, &from.size, &to.size, t);
    color_ref(&mut value.color, &from.color, &to.color, t);
}

fn flow(value: &mut FlowStyle, from: &FlowStyle, to: &FlowStyle, t: f32) {
    opt(&mut value.spacing, &from.spacing, &to.spacing, t);
    opt(&mut value.size, &from.size, &to.size, t);
    color_ref(&mut value.color, &from.color, &to.color, t);
}

fn node_style(value: &mut NodeStyle, from: &NodeStyle, to: &NodeStyle, t: f32) {
    color_ref(&mut value.fill, &from.fill, &to.fill, t);
    opt(&mut value.fill_fade, &from.fill_fade, &to.fill_fade, t);
    text(&mut value.text, &from.text, &to.text, t);
}

fn frame(value: &mut FrameStyle, from: &FrameStyle, to: &FrameStyle, t: f32) {
    opt(&mut value.gap, &from.gap, &to.gap, t);
    opt(&mut value.inset, &from.inset, &to.inset, t);
    text(&mut value.text, &from.text, &to.text, t);
}

fn ring(value: &mut RingStyle, from: &RingStyle, to: &RingStyle, t: f32) {
    text(&mut value.text, &from.text, &to.text, t);
}

fn label(value: &mut LabelStyle, from: &LabelStyle, to: &LabelStyle, t: f32) {
    opt(&mut value.t, &from.t, &to.t, t);
    opt(&mut value.offset, &from.offset, &to.offset, t);
    text(&mut value.text, &from.text, &to.text, t);
}

fn portal_end(value: &mut PortalEndStyle, from: &PortalEndStyle, to: &PortalEndStyle, t: f32) {
    opt(&mut value.border_t, &from.border_t, &to.border_t, t);
    opt(&mut value.offset, &from.offset, &to.offset, t);
    color_ref(&mut value.color, &from.color, &to.color, t);
    opt(&mut value.size, &from.size, &to.size, t);
}

fn path(value: &mut PathData, from: &PathData, to: &PathData, t: f32) {
    flow(&mut value.flow, &from.flow, &to.flow, t);
    if from.control_points.len() == to.control_points.len()
        && value.control_points.len() == from.control_points.len()
    {
        for ((value, from), to) in value
            .control_points
            .iter_mut()
            .zip(&from.control_points)
            .zip(&to.control_points)
        {
            *value = <Vec2 as Interpolate>::lerp(from, to, t);
        }
    }
}

fn theme(value: &mut ThemeData, from: &ThemeData, to: &ThemeData, t: f32) {
    // The active variant's index snaps; the values of variants that
    // keep their shape blend, so editing a variable's color animates.
    if from.variants.len() != to.variants.len() || value.variants.len() != from.variants.len() {
        return;
    }
    for ((value, from), to) in value.variants.iter_mut().zip(&from.variants).zip(&to.variants) {
        if from.values.len() == to.values.len() && value.values.len() == from.values.len() {
            for ((value, from), to) in value.values.iter_mut().zip(&from.values).zip(&to.values) {
                *value = Rgba::lerp(from, to, t);
            }
        }
    }
}

fn palette_group(value: &mut PaletteGroup, from: &PaletteGroup, to: &PaletteGroup, t: f32) {
    plain(&mut value.fill, &from.fill, &to.fill, t);
    plain(&mut value.frame, &from.frame, &to.frame, t);
    plain(&mut value.text, &from.text, &to.text, t);
    if let (Some(value), Some(from), Some(to)) = (value.title.as_mut(), &from.title, &to.title) {
        plain(value, from, to, t);
    }
}

fn defaults(value: &mut DefaultsData, from: &DefaultsData, to: &DefaultsData, t: f32) {
    node_style(&mut value.node, &from.node, &to.node, t);
    text(&mut value.section, &from.section, &to.section, t);
    frame(&mut value.frame, &from.frame, &to.frame, t);
    ring(&mut value.ring, &from.ring, &to.ring, t);
    flow(&mut value.connector, &from.connector, &to.connector, t);
    flow(&mut value.link, &from.link, &to.link, t);
    label(&mut value.label, &from.label, &to.label, t);
    portal_end(&mut value.portal_end, &from.portal_end, &to.portal_end, t);
    text(&mut value.span, &from.span, &to.span, t);
}

/// See the module docs. Cost: a clone of the snapped side, then O(1)
/// per interpolating field (O(n) for control points and theme values).
impl Interpolable for DocElement {
    fn interpolate(before: &Self, after: &Self, t: f32, snap: Snap) -> Self {
        use DocElement as E;
        let mut value = snap.pick(before, after, t).clone();
        match (&mut value, before, after) {
            (E::Map(_), _, _) => {}
            (E::Theme(v), E::Theme(a), E::Theme(b)) => theme(v, a, b, t),
            (E::Palette(v), E::Palette(a), E::Palette(b)) => {
                if a.groups.len() == b.groups.len() && v.groups.len() == a.groups.len() {
                    for ((v, a), b) in v.groups.iter_mut().zip(&a.groups).zip(&b.groups) {
                        palette_group(v, a, b, t);
                    }
                }
            }
            (E::Defaults(v), E::Defaults(a), E::Defaults(b)) => defaults(v, a, b, t),
            (E::Node(v), E::Node(a), E::Node(b)) => {
                v.position = <Vec2 as Interpolate>::lerp(&a.position, &b.position, t);
                opt(&mut v.min_size, &a.min_size, &b.min_size, t);
                node_style(&mut v.style, &a.style, &b.style, t);
                v.opacity = <f32 as Interpolate>::lerp(&a.opacity, &b.opacity, t);
            }
            (E::Section(v), E::Section(a), E::Section(b)) => {
                v.offset = <Vec2 as Interpolate>::lerp(&a.offset, &b.offset, t);
                opt(&mut v.min_size, &a.min_size, &b.min_size, t);
                text(&mut v.text, &a.text, &b.text, t);
            }
            (E::Span(v), E::Span(a), E::Span(b)) => text(&mut v.style, &a.style, &b.style, t),
            (E::Frame(v), E::Frame(a), E::Frame(b)) => frame(&mut v.style, &a.style, &b.style, t),
            (E::Ring(v), E::Ring(a), E::Ring(b)) => ring(&mut v.style, &a.style, &b.style, t),
            (E::Connector(v), E::Connector(a), E::Connector(b)) => path(&mut v.path, &a.path, &b.path, t),
            (E::Link(v), E::Link(a), E::Link(b)) => path(&mut v.path, &a.path, &b.path, t),
            (E::CapStart(v), E::CapStart(a), E::CapStart(b)) | (E::CapEnd(v), E::CapEnd(a), E::CapEnd(b)) => {
                text(&mut v.text, &a.text, &b.text, t)
            }
            (E::Label(v), E::Label(a), E::Label(b)) => label(&mut v.style, &a.style, &b.style, t),
            (E::PortalEnd(v), E::PortalEnd(a), E::PortalEnd(b)) => {
                portal_end(&mut v.style, &a.style, &b.style, t)
            }
            // A change of kind, or of one side's kind: snap whole.
            _ => {}
        }
        value
    }
}
