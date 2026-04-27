// SPDX-License-Identifier: MPL-2.0

//! Text-run invariants: ordered, non-overlapping, within text bounds.
//!
//! `start` and `end` are measured in **grapheme clusters** — what
//! users see as one character — matching how `ColorFontRegions`
//! interprets them in the tree builder and the cosmic-text bridges
//! in `baumhard::font::attrs`. See `lib/baumhard/CONVENTIONS.md §B1`,
//! `CONCEPTS.md`'s `Range` entry, and `format/text-runs.md` for the
//! shared unit contract.

use baumhard::mindmap::model::MindMap;
use baumhard::util::grapheme_chad::count_grapheme_clusters;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for node in map.nodes.values() {
        if node.text_runs.is_empty() {
            continue;
        }

        let total = count_grapheme_clusters(&node.text);
        let mut prev_end: Option<usize> = None;

        for (i, run) in node.text_runs.iter().enumerate() {
            if run.start >= run.end {
                out.push(Violation {
                    category: "text_runs",
                    location: node.id.clone(),
                    message: format!(
                        "run[{}] has start {} not less than end {}",
                        i, run.start, run.end
                    ),
                });
                continue;
            }

            if run.end > total {
                out.push(Violation {
                    category: "text_runs",
                    location: node.id.clone(),
                    message: format!(
                        "run[{}] end {} exceeds text length {} (grapheme clusters)",
                        i, run.end, total
                    ),
                });
            }

            if let Some(p) = prev_end {
                if run.start < p {
                    out.push(Violation {
                        category: "text_runs",
                        location: node.id.clone(),
                        message: format!(
                            "run[{}] overlaps previous run (start {} < previous end {})",
                            i, run.start, p
                        ),
                    });
                }
            }
            prev_end = Some(run.end);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::model::TextRun;
    use crate::verify::test_helpers::node;

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
        n.text = "Hello".into();
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn valid_runs_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hello world".into();
        n.text_runs = vec![run(0, 5), run(6, 11)];
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn overlapping_runs_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hello world".into();
        n.text_runs = vec![run(0, 5), run(3, 8)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "text_runs" && x.message.contains("overlap")));
    }

    #[test]
    fn out_of_bounds_runs_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hi".into();
        n.text_runs = vec![run(0, 100)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "text_runs" && x.message.contains("exceeds")));
    }

    #[test]
    fn inverted_run_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.text = "Hello".into();
        n.text_runs = vec![run(3, 3)];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "text_runs" && x.message.contains("not less than")));
    }

    /// `text_runs` ranges are grapheme-cluster indices, not Unicode
    /// code points. Verifier must measure against the grapheme count;
    /// a region covering one ZWJ-joined emoji family
    /// (`👨‍👩‍👧` — five codepoints, one cluster) at `[0, 1)` is
    /// **valid** and must not raise an "exceeds text length" violation.
    /// Pre-`dc5661a`, the verifier used `chars().count()` and would
    /// have measured 5, so any range > 1 wouldn't have flagged either —
    /// the test here is the positive case the new contract specifies.
    #[test]
    fn zwj_emoji_grapheme_range_passes() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        // 👨‍👩‍👧 = 5 codepoints joined by ZWJ, 1 grapheme cluster.
        n.text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}A".into();
        // Two clusters: the family + the trailing 'A'.
        n.text_runs = vec![run(0, 1), run(1, 2)];
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
        n.text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}A".into();
        n.text_runs = vec![run(0, 5)]; // 5 > 2 clusters
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
