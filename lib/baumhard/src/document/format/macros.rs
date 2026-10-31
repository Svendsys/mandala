// SPDX-License-Identifier: MPL-2.0

//! The views the loader reads a DTO's style keys through.
//!
//! The same five text-style keys sit on a node, a section, a span, a
//! frame, a ring, a cap, a label and six of the defaults, and a few
//! other groups of keys (a frame's, a path's, a portal end's) sit on
//! both an element and its defaults. The DTOs repeat them because they
//! cannot be shared (`flatten` would hide an unknown key), but the
//! conversion to element data is written once, over a view of the
//! fields. These macros build the view from any DTO that has the keys;
//! they only name fields, so a DTO that lacks one fails to compile.

/// `Spelled`, `Serialize` and `Deserialize` for an enum written as one
/// of a table of strings. serde's derive would also accept a unit
/// variant spelled as an object, `{"left": null}`, a second way to write
/// it; this reads a string and nothing else.
macro_rules! spelled {
    ($ty:ident, $what:literal, [$(($name:literal, $variant:path)),* $(,)?]) => {
        impl $crate::document::format::spelling::Spelled for $ty {
            const WHAT: &'static str = $what;
            const NAMES: &'static [(&'static str, Self)] = &[$(($name, $variant)),*];
        }

        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                $crate::document::format::spelling::serialize_spelled(self, serializer)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                $crate::document::format::spelling::deserialize_spelled(deserializer)
            }
        }
    };
}

/// A [`TextIn`](crate::document::format::load_style::TextIn) of `$dto`'s
/// text-style keys.
macro_rules! text_in {
    ($dto:expr) => {
        $crate::document::format::load_style::TextIn {
            font: $dto.font.as_deref(),
            size: $dto.size,
            color: $dto.color.as_ref(),
            weight: $dto.weight,
            italic: $dto.italic,
        }
    };
}

/// A [`FrameIn`](crate::document::format::load_style::FrameIn) of
/// `$dto`'s frame-style keys.
macro_rules! frame_in {
    ($dto:expr) => {
        $crate::document::format::load_style::FrameIn {
            visible: $dto.visible,
            start: $dto.start.as_ref(),
            direction: $dto.direction,
            fit: $dto.fit.as_ref(),
            gap: $dto.gap,
            inset: $dto.inset,
            layout: layout_in!($dto),
            text: text_in!($dto),
        }
    };
}

/// A [`LayoutIn`](crate::document::format::load_style::LayoutIn) of
/// `$dto`'s layout keys: a frame's, a ring's or their defaults'.
macro_rules! layout_in {
    ($dto:expr) => {
        $crate::document::format::load_style::LayoutIn {
            span: $dto.span,
            sides: $dto.sides.as_ref(),
            spacing: $dto.spacing,
        }
    };
}

/// A [`FlowIn`](crate::document::format::load_style::FlowIn) of
/// `$dto`'s path-style keys.
macro_rules! flow_in {
    ($dto:expr) => {
        $crate::document::format::load_style::FlowIn {
            fit: $dto.fit.as_ref(),
            spacing: $dto.spacing,
            size: $dto.size,
            color: $dto.color.as_ref(),
            content: $dto.content.as_deref(),
        }
    };
}

/// A [`PortalIn`](crate::document::format::load_style::PortalIn) of
/// `$dto`'s portal-end style keys.
macro_rules! portal_in {
    ($dto:expr) => {
        $crate::document::format::load_style::PortalIn {
            border_t: $dto.border_t,
            offset: $dto.offset,
            color: $dto.color.as_ref(),
            size: $dto.size,
            content: $dto.content.as_deref(),
        }
    };
}
