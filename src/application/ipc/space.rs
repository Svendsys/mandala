// SPDX-License-Identifier: MPL-2.0

//! The protocol's two named coordinate spaces (`format/ipc.md`
//! §Coordinate spaces). Every command that takes or returns geometry
//! names one, and a command that accepts either takes a `space`
//! parameter spelled with [`Space::parse`]'s words.

use baumhard::gfx_structs::camera::Camera2D;
use serde_json::{json, Value};

use crate::application::document::f32_json;

/// Where a coordinate pair lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Space {
    /// Physical pixels on the rendered surface, origin top-left,
    /// y-down: the space of screenshots and synthetic mouse input.
    Surface,
    /// The document's world space, the space of `MindNode` offsets,
    /// as the camera sees it.
    Canvas,
}

impl Space {
    /// The wire words, for a parameter's `describe` doc and its
    /// error messages.
    pub(crate) const WORDS: &'static str = "\"canvas\" or \"surface\"";

    /// Parse a `space` parameter's value.
    pub(crate) fn parse(word: &str) -> Option<Space> {
        match word {
            "surface" => Some(Space::Surface),
            "canvas" => Some(Space::Canvas),
            _ => None,
        }
    }
}

/// The camera as `query.camera` reports it, and as the `scene` family
/// stamps it on every dump: the `surface` ↔ `canvas` transform
/// (`format/ipc.md` §Coordinate spaces). `center` is the canvas point
/// at the middle of the viewport; a canvas point `p` lands at surface
/// `(p - center) * zoom + surface / 2`.
pub(crate) fn camera_json(camera: &Camera2D) -> Value {
    json!({
        "center": { "x": f32_json(camera.position.x), "y": f32_json(camera.position.y) },
        "zoom": f32_json(camera.zoom),
        "zoom_min": f32_json(Camera2D::MIN_ZOOM),
        "zoom_max": f32_json(Camera2D::MAX_ZOOM),
        "surface": { "width": camera.viewport_size.0, "height": camera.viewport_size.1 },
    })
}

/// A position in a named space: where a synthetic pointer stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) space: Space,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_json_names_the_transform_and_the_zoom_clamp() {
        let mut camera = Camera2D::new(1920, 1080);
        camera.position = glam::Vec2::new(12.0, -340.5);
        camera.zoom = 0.75;
        assert_eq!(
            camera_json(&camera),
            json!({
                "center": { "x": 12.0, "y": -340.5 },
                "zoom": 0.75,
                "zoom_min": 0.05,
                "zoom_max": 5.0,
                "surface": { "width": 1920, "height": 1080 },
            })
        );
    }

    #[test]
    fn test_space_parses_exactly_the_documented_words() {
        assert_eq!(Space::parse("canvas"), Some(Space::Canvas));
        assert_eq!(Space::parse("surface"), Some(Space::Surface));
        for rejected in ["Canvas", "screen", "world", ""] {
            assert_eq!(Space::parse(rejected), None, "{rejected:?}");
        }
    }

    /// The document names both words in its coordinate-space section;
    /// a third space added here without the document fails.
    #[test]
    fn test_space_words_match_the_documented_spaces() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};

        let section = section_text(&repo_path("format/ipc.md"), "### Coordinate spaces");
        for word in ["surface", "canvas"] {
            assert!(
                section.contains(&format!("**`{word}`**")),
                "format/ipc.md §Coordinate spaces must define `{word}`"
            );
            assert!(Space::WORDS.contains(&format!("\"{word}\"")));
        }
    }
}
