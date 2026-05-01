// SPDX-License-Identifier: MPL-2.0

//! Text-run invariants: ordered, non-overlapping, within text
//! bounds. `start`/`end` are grapheme clusters (see
//! `format/text-runs.md`).

use baumhard::mindmap::model::MindMap;
use baumhard::util::grapheme_chad::count_grapheme_clusters;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for (_loc, node) in map.node_locations() {
        for (s_idx, section) in node.sections.iter().enumerate() {
            if section.text_runs.is_empty() {
                continue;
            }

            let total = count_grapheme_clusters(&section.text);
            let mut prev_end: Option<usize> = None;

            for (i, run) in section.text_runs.iter().enumerate() {
                if run.start >= run.end {
                    out.push(Violation::node(
                        "text_runs",
                        node,
                        format!(
                            "section[{}].run[{}] has start {} not less than end {}",
                            s_idx, i, run.start, run.end
                        ),
                    ));
                    continue;
                }

                if run.end > total {
                    out.push(Violation::node(
                        "text_runs",
                        node,
                        format!(
                            "section[{}].run[{}] end {} exceeds text length {} (grapheme clusters)",
                            s_idx, i, run.end, total
                        ),
                    ));
                }

                if let Some(p) = prev_end {
                    if run.start < p {
                        out.push(Violation::node(
                            "text_runs",
                            node,
                            format!(
                                "section[{}].run[{}] overlaps previous run (start {} < previous end {})",
                                s_idx, i, run.start, p
                            ),
                        ));
                    }
                }
                prev_end = Some(run.end);
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;
    use baumhard::mindmap::model::TextRun;

    fn run(start: usize, end: usize) -> TextRun {
        TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14,
            color: "#ffffff".into(),
            hyperlink: None,
        }
    }

    #[test]
    fn empty_runs_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0].text = "Hello".into();
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn valid_runs_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0].text = "Hello world".into();
        n.sections[0].text_runs = vec![run(0, 5), run(6, 11)];
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn overlapping_runs_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0].text = "Hello world".into();
        n.sections[0].text_runs = vec![run(0, 5), run(3, 8)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "text_runs" && x.message.contains("overlap")));
    }

    #[test]
    fn out_of_bounds_runs_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0].text = "Hi".into();
        n.sections[0].text_runs = vec![run(0, 100)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "text_runs" && x.message.contains("exceeds")));
    }

    #[test]
    fn inverted_run_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0].text = "Hello".into();
        n.sections[0].text_runs = vec![run(3, 3)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "text_runs" && x.message.contains("not less than")));
    }

    /// ZWJ family at `[0,1)` is one grapheme cluster — must not flag
    /// "exceeds text length". Locks the grapheme-cluster contract.
    #[test]
    fn zwj_emoji_grapheme_range_passes() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        // 👨‍👩‍👧 = 5 codepoints joined by ZWJ, 1 grapheme cluster.
        n.sections[0].text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}A".into();
        // Two clusters: the family + the trailing 'A'.
        n.sections[0].text_runs = vec![run(0, 1), run(1, 2)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(
            v.is_empty(),
            "two grapheme-cluster runs over a 2-cluster string should pass; got {:?}",
            v
        );
    }

    /// The same emoji-bearing string with a range past the cluster
    /// count must flag with the new "(grapheme clusters)" suffix —
    /// proves the error message migrated alongside the unit.
    #[test]
    fn zwj_emoji_out_of_bounds_uses_grapheme_unit_in_message() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0].text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}A".into();
        n.sections[0].text_runs = vec![run(0, 5)]; // 5 > 2 clusters
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        let exceeded = v
            .iter()
            .find(|x| x.category == "text_runs" && x.message.contains("exceeds"))
            .expect("expected an out-of-bounds violation");
        assert!(
            exceeded.message.contains("grapheme clusters"),
            "error message must name the new unit; got: {}",
            exceeded.message
        );
        assert!(
            !exceeded.message.contains("code points"),
            "error message must drop the old unit; got: {}",
            exceeded.message
        );
    }
}
