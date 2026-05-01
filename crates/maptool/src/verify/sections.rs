// SPDX-License-Identifier: MPL-2.0

//! Section-bounds invariants. The `MindNode` docstring promises
//! `maptool verify` flags out-of-bounds sections; this module
//! delivers on that promise. Checks per [`MindSection`]:
//!
//! - `offset.{x,y}` finite and non-negative,
//! - `size.{width,height}` (when set) finite and strictly positive,
//! - `offset + size` (when size set) inside the parent node's
//!   `size` AABB.
//!
//! Violations are emitted with the parent node's id as location
//! and the offending section index inlined into the message, so
//! a multi-section node still pinpoints which section failed.

use baumhard::mindmap::model::MindMap;

use super::Violation;

const CATEGORY: &str = "sections";

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for (_loc, node) in map.node_locations() {
        for (s_idx, section) in node.sections.iter().enumerate() {
            check_offset_finite(node, s_idx, section, &mut out);
            check_offset_non_negative(node, s_idx, section, &mut out);

            if let Some(size) = section.size.as_ref() {
                check_size_finite(node, s_idx, size, &mut out);
                check_size_positive(node, s_idx, size, &mut out);
                check_within_node_aabb(node, s_idx, section, size, &mut out);
            }
        }
    }

    out
}

fn check_offset_finite(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    section: &baumhard::mindmap::model::MindSection,
    out: &mut Vec<Violation>,
) {
    if !section.offset.x.is_finite() || !section.offset.y.is_finite() {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}].offset has non-finite component (x={}, y={})",
                s_idx, section.offset.x, section.offset.y
            ),
        ));
    }
}

fn check_offset_non_negative(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    section: &baumhard::mindmap::model::MindSection,
    out: &mut Vec<Violation>,
) {
    if section.offset.x.is_finite() && section.offset.x < 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("section[{}].offset.x is negative ({})", s_idx, section.offset.x),
        ));
    }
    if section.offset.y.is_finite() && section.offset.y < 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("section[{}].offset.y is negative ({})", s_idx, section.offset.y),
        ));
    }
}

fn check_size_finite(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    size: &baumhard::mindmap::model::Size,
    out: &mut Vec<Violation>,
) {
    if !size.width.is_finite() || !size.height.is_finite() {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}].size has non-finite component (width={}, height={})",
                s_idx, size.width, size.height
            ),
        ));
    }
}

fn check_size_positive(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    size: &baumhard::mindmap::model::Size,
    out: &mut Vec<Violation>,
) {
    if size.width.is_finite() && size.width <= 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("section[{}].size.width is not positive ({})", s_idx, size.width),
        ));
    }
    if size.height.is_finite() && size.height <= 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("section[{}].size.height is not positive ({})", s_idx, size.height),
        ));
    }
}

fn check_within_node_aabb(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    section: &baumhard::mindmap::model::MindSection,
    size: &baumhard::mindmap::model::Size,
    out: &mut Vec<Violation>,
) {
    if !section.offset.x.is_finite() || !section.offset.y.is_finite()
        || !size.width.is_finite() || !size.height.is_finite()
    {
        return;
    }

    let right = section.offset.x + size.width;
    let bottom = section.offset.y + size.height;
    if right > node.size.width {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}] extends past node right edge ({} > {})",
                s_idx, right, node.size.width
            ),
        ));
    }
    if bottom > node.size.height {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}] extends past node bottom edge ({} > {})",
                s_idx, bottom, node.size.height
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;
    use baumhard::mindmap::model::{MindSection, Position, Size};

    fn section(offset: Position, size: Option<Size>) -> MindSection {
        MindSection {
            text: String::new(),
            text_runs: Vec::new(),
            offset,
            size,
            channel: 0,
            trigger_bindings: Vec::new(),
        }
    }

    #[test]
    fn default_section_clean() {
        let mut map = MindMap::new_blank("t");
        let n = node("0", None);
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn explicit_within_aabb_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 5.0, y: 5.0 },
            Some(Size { width: 50.0, height: 20.0 }),
        );
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn flush_to_node_aabb_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size { width: 100.0, height: 40.0 }),
        );
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn negative_offset_x_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(Position { x: -1.0, y: 0.0 }, None);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == CATEGORY && x.message.contains("offset.x is negative")));
    }

    #[test]
    fn negative_offset_y_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(Position { x: 0.0, y: -2.0 }, None);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == CATEGORY && x.message.contains("offset.y is negative")));
    }

    #[test]
    fn nan_offset_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(Position { x: f64::NAN, y: 0.0 }, None);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == CATEGORY && x.message.contains("non-finite")));
    }

    #[test]
    fn zero_size_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size { width: 0.0, height: 10.0 }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == CATEGORY && x.message.contains("size.width is not positive")));
    }

    #[test]
    fn negative_size_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size { width: 10.0, height: -5.0 }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == CATEGORY && x.message.contains("size.height is not positive")));
    }

    #[test]
    fn nan_size_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size { width: f64::NAN, height: 10.0 }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == CATEGORY && x.message.contains("non-finite")));
    }

    #[test]
    fn extends_past_right_edge_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 50.0, y: 0.0 },
            Some(Size { width: 60.0, height: 10.0 }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == CATEGORY && x.message.contains("past node right edge")));
    }

    #[test]
    fn extends_past_bottom_edge_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 30.0 },
            Some(Size { width: 10.0, height: 20.0 }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == CATEGORY && x.message.contains("past node bottom edge")));
    }

    #[test]
    fn multi_section_pinpoints_offending_index() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections = vec![
            section(Position { x: 0.0, y: 0.0 }, None),
            section(Position { x: -3.0, y: 0.0 }, None),
        ];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        let off = v.iter().find(|x| x.message.contains("offset.x is negative")).expect("missed violation");
        assert!(off.message.contains("section[1]"), "expected section index 1 in message: {}", off.message);
    }

    #[test]
    fn unset_size_skips_aabb_check() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(Position { x: 99.0, y: 39.0 }, None);
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty(), "size=None means \"fill node\"; offsets without size are not bounds-checked");
    }
}
