// SPDX-License-Identifier: MPL-2.0

//! The document's predicate surface:
//! [`DocField`](crate::document::inspect::DocField), the facets of a
//! [`DocElement`](crate::document::element::DocElement) a
//! [`Predicate`](crate::engine::predicate::Predicate) can test, and how
//! an element answers each one.
//!
//! A predicate tests what an element **stores**, not what it resolves
//! to: a node with no `fill` of its own has no fill here, whatever its
//! palette or the map's defaults would color it. Resolution needs the
//! tree, and a predicate sees one element (design §4.3).
//!
//! The rules, the same for every field:
//!
//! - A field the element's kind does not carry answers `None`, and the
//!   predicate moves on to its next pair.
//! - `Exists(false)` passes when the element stores a value — always,
//!   for a field that is not optional — and `Exists(true)` when it does
//!   not.
//! - `Equals(false)` is `==` and `Equals(true)` is `!=`; an unset
//!   optional field equals no value. Numbers compare with
//!   [`Comparator::compare_f32`](crate::engine::predicate::Comparator::compare_f32)'s tolerance.
//! - `GreaterThan` and `LessThan` order numbers — and font sizes of the
//!   same unit — and fail on everything else, negated or not, as they
//!   do on an unset field: there is nothing to order.

use crate::document::element::{DocElement, DocKind, TextStyle};
use crate::document::values::{Align, Anchor, ColorRef, Direction, Edge, FitMode, FontSize, LinkKind, Shape};
use crate::engine::predicate::{Comparator, Inspect};
use crate::font::fonts::AppFont;
use serde::{Deserialize, Serialize};

/// A facet of a document element a predicate can test, with the value
/// it is compared against. Serialized in `snake_case`:
/// `{"kind": "node"}`, `{"opacity": 0.5}`, `{"fill": {"rgba": [255, 0, 0, 255]}}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocField {
    /// The element's kind. Every element.
    Kind(DocKind),
    /// The element's channel. Every element.
    Channel(usize),
    /// A node's ident.
    Ident(u32),
    /// A node's horizontal position.
    PositionX(f32),
    /// A node's vertical position.
    PositionY(f32),
    /// A node's or a section's minimum width.
    MinWidth(f32),
    /// A node's or a section's minimum height.
    MinHeight(f32),
    /// A node's shape.
    Shape(Shape),
    /// A node's fill.
    Fill(ColorRef),
    /// A node's fill fade.
    FillFade(f32),
    /// The index of a node's palette.
    Palette(u16),
    /// Whether a node is folded.
    Folded(bool),
    /// Whether a node is hidden.
    Hidden(bool),
    /// A node's opacity.
    Opacity(f32),
    /// The index of the verb a node's click runs.
    OnClick(u16),
    /// The index of the verb hovering a node runs.
    OnHover(u16),
    /// The lowest zoom a node, connector or link renders at.
    ZoomMin(f32),
    /// The highest zoom a node, connector or link renders at.
    ZoomMax(f32),
    /// A section's horizontal offset.
    OffsetX(f32),
    /// A section's vertical offset.
    OffsetY(f32),
    /// A section's alignment.
    Align(Align),
    /// The face of an element's text style: a node's, section's,
    /// span's, frame's, ring's, cap's or label's.
    Font(AppFont),
    /// The text size: a text style's, a path's (in points) or a portal
    /// end's (in points).
    FontSize(FontSize),
    /// The text color: a text style's, a path's or a portal end's.
    Color(ColorRef),
    /// The weight of an element's text style, `100..=900`.
    Weight(u16),
    /// Whether an element's text style is italic.
    Italic(bool),
    /// Whether a frame, connector or link is drawn.
    Visible(bool),
    /// The side a frame's text starts on.
    StartEdge(Edge),
    /// How far along that side a frame's text starts.
    StartAt(f32),
    /// Which way a frame's text runs.
    Direction(Direction),
    /// The fit mode of a frame, ring, connector or link.
    Fit(FitMode),
    /// The space between a frame's rings.
    Gap(f32),
    /// A frame's distance from the outline.
    Inset(f32),
    /// The extra space between a path's glyphs.
    Spacing(f32),
    /// Where a path attaches at its start.
    AnchorFrom(Anchor),
    /// Where a path attaches at its end.
    AnchorTo(Anchor),
    /// Whether a link draws a path or a portal.
    LinkKind(LinkKind),
    /// A label's position along its path.
    LabelT(f32),
    /// A label's or a portal end's distance from its path or outline.
    Offset(f32),
    /// A portal end's position around its node's outline.
    BorderT(f32),
}

/// A stored value, as a comparison sees it.
enum Stored<'a, T> {
    /// The element's kind does not carry the field.
    Absent,
    /// The element carries the field, and it holds this, or nothing.
    Value(Option<&'a T>),
}

/// `Exists` and `Equals` on a value with no order. O(1).
fn unordered<T: PartialEq>(stored: Stored<'_, T>, reference: &T, comparator: &Comparator) -> Option<bool> {
    let Stored::Value(value) = stored else {
        return None;
    };
    Some(match comparator {
        Comparator::Exists(negation) => value.is_some() != *negation,
        Comparator::Equals(negation) => (value == Some(reference)) != *negation,
        Comparator::GreaterThan(_) | Comparator::LessThan(_) => false,
    })
}

/// Every comparator on a number, through [`Comparator::compare_f32`].
/// O(1).
fn number(stored: Stored<'_, f32>, reference: f32, comparator: &Comparator) -> Option<bool> {
    let Stored::Value(value) = stored else {
        return None;
    };
    Some(match (comparator, value) {
        (Comparator::Exists(negation), _) => value.is_some() != *negation,
        (Comparator::Equals(negation), None) => *negation,
        (_, None) => false,
        (_, Some(value)) => comparator.compare_f32(*value, reference),
    })
}

/// Every comparator on an integer. O(1).
fn integer<T: Copy + Ord>(stored: Stored<'_, T>, reference: T, comparator: &Comparator) -> Option<bool> {
    let Stored::Value(value) = stored else {
        return None;
    };
    Some(match (comparator, value) {
        (Comparator::Exists(negation), _) => value.is_some() != *negation,
        (Comparator::Equals(negation), None) => *negation,
        (_, None) => false,
        (Comparator::Equals(negation), Some(value)) => (*value == reference) != *negation,
        (Comparator::GreaterThan(negation), Some(value)) => (*value > reference) != *negation,
        (Comparator::LessThan(negation), Some(value)) => (*value < reference) != *negation,
    })
}

/// Font sizes: equal when unit and number agree; ordered within one
/// unit only. O(1).
fn font_size(stored: Stored<'_, FontSize>, reference: FontSize, comparator: &Comparator) -> Option<bool> {
    let Stored::Value(value) = stored else {
        return None;
    };
    Some(match (comparator, value) {
        (Comparator::Exists(negation), _) => value.is_some() != *negation,
        (Comparator::Equals(negation), None) => *negation,
        (_, None) => false,
        (_, Some(value)) => match (*value, reference) {
            (FontSize::Pt(a), FontSize::Pt(b)) | (FontSize::Rel(a), FontSize::Rel(b)) => {
                comparator.compare_f32(a, b)
            }
            _ => matches!(comparator, Comparator::Equals(true)),
        },
    })
}

/// The element's text style, or [`Stored::Absent`]. O(1).
fn text<'a, T>(element: &'a DocElement, pick: impl Fn(&'a TextStyle) -> Option<&'a T>) -> Stored<'a, T> {
    match element.text_style() {
        Some(style) => Stored::Value(pick(style)),
        None => Stored::Absent,
    }
}

/// `Some(value)` when `present`, as [`Stored::Value`]; else
/// [`Stored::Absent`].
fn stored<T>(value: Option<Option<&T>>) -> Stored<'_, T> {
    match value {
        Some(value) => Stored::Value(value),
        None => Stored::Absent,
    }
}

impl Inspect for DocElement {
    type Field = DocField;

    /// See the module docs for the rules. O(1); no allocation.
    fn test_field(&self, field: &DocField, comparator: &Comparator) -> Option<bool> {
        let node = self.as_node();
        let path = self.as_path();
        let section = match self {
            DocElement::Section(section) => Some(section),
            _ => None,
        };
        let frame = match self {
            DocElement::Frame(frame) => Some(&frame.style),
            _ => None,
        };
        let portal_end = match self {
            DocElement::PortalEnd(end) => Some(&end.style),
            _ => None,
        };
        let label = match self {
            DocElement::Label(label) => Some(&label.style),
            _ => None,
        };
        let zoom = node.map(|n| &n.zoom).or_else(|| path.map(|p| &p.zoom));
        let min_size = node.map(|n| &n.min_size).or_else(|| section.map(|s| &s.min_size));
        match field {
            DocField::Kind(kind) => unordered(Stored::Value(Some(&self.kind())), kind, comparator),
            DocField::Channel(channel) => integer(Stored::Value(Some(&self.channel())), *channel, comparator),
            DocField::Ident(ident) => integer(stored(node.map(|n| Some(&n.ident.0))), *ident, comparator),
            DocField::PositionX(x) => number(stored(node.map(|n| Some(&n.position.x))), *x, comparator),
            DocField::PositionY(y) => number(stored(node.map(|n| Some(&n.position.y))), *y, comparator),
            DocField::MinWidth(w) => {
                number(stored(min_size.map(|s| s.as_ref().map(|s| &s.x))), *w, comparator)
            }
            DocField::MinHeight(h) => {
                number(stored(min_size.map(|s| s.as_ref().map(|s| &s.y))), *h, comparator)
            }
            DocField::Shape(shape) => {
                unordered(stored(node.map(|n| n.style.shape.as_ref())), shape, comparator)
            }
            DocField::Fill(fill) => unordered(stored(node.map(|n| n.style.fill.as_ref())), fill, comparator),
            DocField::FillFade(fade) => number(
                stored(node.map(|n| n.style.fill_fade.as_ref())),
                *fade,
                comparator,
            ),
            DocField::Palette(palette) => integer(
                stored(node.map(|n| n.palette.as_ref().map(|p| &p.0))),
                *palette,
                comparator,
            ),
            DocField::Folded(folded) => unordered(stored(node.map(|n| Some(&n.folded))), folded, comparator),
            DocField::Hidden(hidden) => unordered(stored(node.map(|n| Some(&n.hidden))), hidden, comparator),
            DocField::Opacity(opacity) => {
                number(stored(node.map(|n| Some(&n.opacity))), *opacity, comparator)
            }
            DocField::OnClick(verb) => integer(
                stored(node.map(|n| n.on.click.as_ref().map(|v| &v.0))),
                *verb,
                comparator,
            ),
            DocField::OnHover(verb) => integer(
                stored(node.map(|n| n.on.hover.as_ref().map(|v| &v.0))),
                *verb,
                comparator,
            ),
            DocField::ZoomMin(min) => number(
                stored(zoom.map(|z| z.as_ref().and_then(|z| z.min.as_ref()))),
                *min,
                comparator,
            ),
            DocField::ZoomMax(max) => number(
                stored(zoom.map(|z| z.as_ref().and_then(|z| z.max.as_ref()))),
                *max,
                comparator,
            ),
            DocField::OffsetX(x) => number(stored(section.map(|s| Some(&s.offset.x))), *x, comparator),
            DocField::OffsetY(y) => number(stored(section.map(|s| Some(&s.offset.y))), *y, comparator),
            DocField::Align(align) => unordered(stored(section.map(|s| s.align.as_ref())), align, comparator),
            DocField::Font(font) => unordered(text(self, |t| t.font.as_ref()), font, comparator),
            DocField::FontSize(size) => {
                let flow_size = path.map(|p| p.flow.size).or_else(|| portal_end.map(|e| e.size));
                match flow_size {
                    Some(points) => font_size(
                        Stored::Value(points.map(FontSize::Pt).as_ref()),
                        *size,
                        comparator,
                    ),
                    None => font_size(text(self, |t| t.size.as_ref()), *size, comparator),
                }
            }
            DocField::Color(color) => {
                let flow_color = path.map(|p| p.flow.color).or_else(|| portal_end.map(|e| e.color));
                match flow_color {
                    Some(stored_color) => unordered(Stored::Value(stored_color.as_ref()), color, comparator),
                    None => unordered(text(self, |t| t.color.as_ref()), color, comparator),
                }
            }
            DocField::Weight(weight) => {
                let stored_weight = self.text_style().map(|t| t.weight.map(|w| w.get()));
                match stored_weight {
                    Some(value) => integer(Stored::Value(value.as_ref()), *weight, comparator),
                    None => None,
                }
            }
            DocField::Italic(italic) => unordered(text(self, |t| t.italic.as_ref()), italic, comparator),
            DocField::Visible(visible) => {
                let stored_visible = frame
                    .map(|f| f.visible.as_ref())
                    .or_else(|| path.map(|p| Some(&p.visible)));
                unordered(stored(stored_visible), visible, comparator)
            }
            DocField::StartEdge(edge) => unordered(
                stored(frame.map(|f| f.start.as_ref().map(|s| &s.edge))),
                edge,
                comparator,
            ),
            DocField::StartAt(at) => number(
                stored(frame.map(|f| f.start.as_ref().map(|s| &s.at))),
                *at,
                comparator,
            ),
            DocField::Direction(direction) => {
                unordered(stored(frame.map(|f| f.direction.as_ref())), direction, comparator)
            }
            DocField::Fit(fit) => {
                let stored_fit = match self {
                    DocElement::Frame(frame) => Some(frame.style.fit.as_ref()),
                    DocElement::Ring(ring) => Some(ring.style.fit.as_ref()),
                    _ => path.map(|p| p.flow.fit.as_ref()),
                };
                unordered(stored(stored_fit), fit, comparator)
            }
            DocField::Gap(gap) => number(stored(frame.map(|f| f.gap.as_ref())), *gap, comparator),
            DocField::Inset(inset) => number(stored(frame.map(|f| f.inset.as_ref())), *inset, comparator),
            DocField::Spacing(spacing) => number(
                stored(path.map(|p| p.flow.spacing.as_ref())),
                *spacing,
                comparator,
            ),
            DocField::AnchorFrom(anchor) => {
                unordered(stored(path.map(|p| Some(&p.anchors.0))), anchor, comparator)
            }
            DocField::AnchorTo(anchor) => {
                unordered(stored(path.map(|p| Some(&p.anchors.1))), anchor, comparator)
            }
            DocField::LinkKind(kind) => {
                let stored_kind = match self {
                    DocElement::Link(link) => Some(Some(&link.kind)),
                    _ => None,
                };
                unordered(stored(stored_kind), kind, comparator)
            }
            DocField::LabelT(t) => number(stored(label.map(|l| l.t.as_ref())), *t, comparator),
            DocField::Offset(offset) => {
                let stored_offset = label
                    .map(|l| l.offset.as_ref())
                    .or_else(|| portal_end.map(|e| e.offset.as_ref()));
                number(stored(stored_offset), *offset, comparator)
            }
            DocField::BorderT(t) => number(stored(portal_end.map(|e| e.border_t.as_ref())), *t, comparator),
        }
    }
}
