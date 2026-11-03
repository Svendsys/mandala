// SPDX-License-Identifier: MPL-2.0

//! Tree-builder node-container tests: the container draws nothing.

use super::super::*;
use super::fixtures::*;

/// A node has no background fill (V1-B12): its container area is the
/// same whatever `style.background_color` says — a color, a theme
/// variable, nothing at all — and it draws nothing, holding no text.
/// The control is a field the container does carry: a different shape
/// is a different container, so "the same" is not a comparison that
/// cannot fail.
#[test]
fn test_a_node_container_draws_no_background() {
    let container = |edit: &dyn Fn(&mut crate::mindmap::model::MindNode)| {
        let mut map = synthetic_map(vec![synthetic_node("n", None, 0.0, 0.0)], vec![]);
        map.canvas
            .theme_variables
            .insert("--panel".into(), "#112233".into());
        edit(map.nodes.get_mut("n").unwrap());
        let result = build_mindmap_tree(&map);
        glyph_area_of(&result.tree, result.arena_id_for("n").unwrap()).clone()
    };
    let plain = container(&|_| {});
    assert!(
        plain.text.is_empty(),
        "the container holds no text: it draws nothing"
    );
    for background in ["#ff8800", "var(--panel)", "", "#00000000", "not-a-color"] {
        let with = container(&|node| node.style.background_color = background.into());
        assert!(
            with.same_state(&plain),
            "{background:?}: a node background changes nothing the container draws"
        );
    }
    let ellipse = container(&|node| node.style.shape = "ellipse".into());
    assert!(
        !ellipse.same_state(&plain),
        "the control: the shape is the container's, and changes it"
    );
}
