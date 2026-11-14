// SPDX-License-Identifier: MPL-2.0

//! `verify`'s warnings: what the loader accepts but is probably a
//! mistake.
//!
//! The hard failures are the loader's alone. `verify` loads the map with
//! [`file::read`](baumhard::document::format::file::read), and a refusal
//! is printed as the loader words it, with its JSON path; nothing here
//! judges what the loader judges. Once a map has loaded, [`warnings`]
//! looks for four things:
//!
//! - **A verb nothing refers to.** A verb is referred to by a node's `on`
//!   (`click`, `hover`) and by a span's `link` (`{"verb": …}`), and by
//!   nothing else in the format.
//! - **A node saved hidden that nothing can reveal.** What reveals a node
//!   is a step of a presentation or a verb, and no definition the format
//!   holds yet has steps: an edit, a presentation and a verb are an id
//!   and a name until V1-D1 to V1-D3 (#250–#252) complete them. So today
//!   every node saved with `"hidden": true` is reported.
//! - **An empty frame**: one that resolves visible, through its own
//!   `visible`, the map's defaults and the built-in value, and holds no
//!   rings — it draws nothing.
//! - **A portal link whose ends both have no content**: each end has no
//!   spans and no `content` of its own, and the content it resolves to
//!   through the map's defaults is empty, so neither end draws a marker.
//!
//! Each warning names its place as `grep` does: `#<id>` and the path
//! below the node, or a definition's path for a verb.

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind};
use baumhard::document::ids::Document;
use baumhard::document::resolve::{resolve_frame, resolve_portal_end, Content};
use baumhard::document::values::{LinkKind, LinkTarget, VerbId};

use crate::map::{ident, is, link, text_owners, Nodes};

/// Every warning `doc` earns, in the file's order: the nodes' in
/// pre-order — each node's own, then its frame's, then its links' — then
/// the verbs', in the order `verbs` lists them.
///
/// Cost: one walk of the nodes and every element below them, and one
/// flag per verb.
pub fn warnings(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    let mut referenced = vec![false; doc.definitions().verbs.len()];
    let mut refer = |verb: VerbId| {
        if let Some(flag) = referenced.get_mut(usize::from(verb.0)) {
            *flag = true;
        }
    };
    for (node, _) in Nodes::new(doc) {
        let id = ident(doc, node);
        let Some(data) = doc.get(node).and_then(DocElement::as_node) else {
            continue;
        };
        data.on
            .click
            .into_iter()
            .chain(data.on.hover)
            .for_each(&mut refer);
        if data.hidden {
            out.push(format!(
                "#{id}: hidden, and no verb or presentation can reveal it"
            ));
        }
        text_owners(doc, node, |_, owner| {
            for span in doc.children(owner) {
                if let Some(DocElement::Span(span)) = doc.get(span) {
                    if let Some(LinkTarget::Verb(verb)) = span.link {
                        refer(verb);
                    }
                }
            }
        });
        let mut links = 0;
        for child in doc.children(node) {
            if is(doc, child, DocKind::Frame) && empty_frame(doc, child) {
                out.push(format!("#{id} frame: visible with no rings"));
            }
            if let Some(link) = link(doc, child) {
                if link.kind == LinkKind::Portal && doc.children(child).all(|end| empty_end(doc, end)) {
                    out.push(format!(
                        "#{id} links[{links}]: a portal whose ends both have no content"
                    ));
                }
                links += 1;
            }
        }
    }
    for (n, verb) in doc.definitions().verbs.iter().enumerate() {
        if !referenced[n] {
            out.push(format!(
                "verbs[{n}] {:?}: defined, and nothing refers to it",
                verb.id
            ));
        }
    }
    out
}

/// Whether the frame `frame` resolves visible and holds no rings.
fn empty_frame(doc: &Document, frame: NodeId) -> bool {
    resolve_frame(doc.tree(), frame, &mut ()).visible
        && !doc.children(frame).any(|id| is(doc, id, DocKind::Ring))
}

/// Whether the portal end `end` has no spans, no `content` of its own,
/// and resolves to empty content.
fn empty_end(doc: &Document, end: NodeId) -> bool {
    let Some(DocElement::PortalEnd(data)) = doc.get(end) else {
        return false;
    };
    data.style.content.is_none()
        && !doc.children(end).any(|id| is(doc, id, DocKind::Span))
        && matches!(resolve_portal_end(doc.tree(), end, &mut ()).content, Content::Text(text) if text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::document::format;

    /// The warnings of the map whose top-level keys are `header` (a
    /// comma-terminated list, or nothing) and whose nodes are `nodes`.
    fn warned(header: &str, nodes: &str) -> Vec<String> {
        let doc = format::load(&format!(
            r#"{{"format": 1, "name": "m", {header} "nodes": [{nodes}]}}"#
        ))
        .unwrap_or_else(|e| panic!("the map loads: {e}"));
        warnings(&doc)
    }

    /// A verb nothing refers to is reported by its place in `verbs`; one
    /// a node's `on` or a span's `link` names is not.
    ///
    /// Control: not calling `refer` for the triggers fails the `on` case,
    /// and not calling it for spans fails the span case — each with the
    /// `verbs[0]` warning the case expects to be absent.
    #[test]
    fn test_verify_warns_of_a_verb_nothing_refers_to() {
        let verbs = r#""verbs": [{"id": "go", "name": "Go"}, {"id": "stay", "name": "Stay"}],"#;
        let plain = r#"{"id": 1, "position": [0, 0], "sections": [{"spans": [{"text": "a"}]}]}"#;
        assert_eq!(
            warned(verbs, plain),
            [
                r#"verbs[0] "go": defined, and nothing refers to it"#,
                r#"verbs[1] "stay": defined, and nothing refers to it"#
            ]
        );
        let by_trigger = r#"{"id": 1, "position": [0, 0], "on": {"hover": "go"}}"#;
        assert_eq!(
            warned(verbs, by_trigger),
            [r#"verbs[1] "stay": defined, and nothing refers to it"#]
        );
        let by_span = r#"{"id": 1, "position": [0, 0], "links": [{"to": 2, "kind": "portal", "ends": [{}, {"label": {"spans": [{"text": "b", "link": {"verb": "go"}}]}}]}]},
                         {"id": 2, "position": [0, 0]}"#;
        assert_eq!(
            warned(verbs, by_span),
            [r#"verbs[1] "stay": defined, and nothing refers to it"#]
        );
    }

    /// A node saved hidden is reported; the same node not hidden, and its
    /// child (hidden through it, not saved hidden), are not.
    ///
    /// Control: testing `folded` in place of `hidden` makes the second
    /// assertion report node 4.
    #[test]
    fn test_verify_warns_of_a_hidden_node() {
        let nodes = |hidden: bool| {
            format!(
                r#"{{"id": 4, "position": [0, 0], "hidden": {hidden}, "folded": true}}, {{"id": 5, "parent": 4, "position": [0, 0]}}"#
            )
        };
        assert_eq!(
            warned("", &nodes(true)),
            ["#4: hidden, and no verb or presentation can reveal it"]
        );
        assert!(warned("", &nodes(false)).is_empty());
    }

    /// A frame with no rings is reported when it resolves visible —
    /// unset, through the built-in value, or its own `true` over the
    /// defaults' `false` — and not when its own `visible` or the map's
    /// defaults hide it, nor when it has a ring.
    ///
    /// Control: reading only the frame's own `visible` (`== Some(true)`)
    /// in place of the resolved value fails the first assertion; reading
    /// it with `unwrap_or(true)`, ignoring the defaults, fails the third.
    #[test]
    fn test_verify_warns_of_a_visible_frame_with_no_rings() {
        let node = |frame: &str| format!(r#"{{"id": 2, "position": [0, 0], "frame": {frame}}}"#);
        let hidden_by_default = r#""defaults": {"frame": {"visible": false}},"#;
        assert_eq!(warned("", &node("{}")), ["#2 frame: visible with no rings"]);
        assert!(warned("", &node(r#"{"visible": false}"#)).is_empty());
        assert!(warned(hidden_by_default, &node("{}")).is_empty());
        assert_eq!(
            warned(hidden_by_default, &node(r#"{"visible": true}"#)),
            ["#2 frame: visible with no rings"]
        );
        assert!(warned("", &node(r#"{"rings": [{}]}"#)).is_empty());
    }

    /// A portal whose ends both draw nothing is reported by its place
    /// among the node's links. Not reported: the same ends when the
    /// defaults leave the content unset (the built-in marker applies), an
    /// end with spans, and an end with `content` of its own.
    ///
    /// Control: testing one end in place of both (`any`) fails the case
    /// where only the first end has spans; dropping the own-`content`
    /// test fails the last case, whose own empty content resolves empty.
    #[test]
    fn test_verify_warns_of_a_portal_whose_ends_have_no_content() {
        let empty = r#""defaults": {"portal_end": {"content": ""}},"#;
        let nodes = |ends: &str| {
            format!(
                r#"{{"id": 1, "position": [0, 0], "links": [{{"to": 2, "kind": "line"}}, {{"to": 2, "kind": "portal", "ends": {ends}}}]}},
                   {{"id": 2, "position": [0, 0]}}"#
            )
        };
        assert_eq!(
            warned(
                empty,
                &nodes(r#"[{}, {"label": {"spans": [{"text": "a label is not a marker"}]}}]"#)
            ),
            ["#1 links[1]: a portal whose ends both have no content"]
        );
        assert!(warned("", &nodes("[{}, {}]")).is_empty());
        assert!(warned(empty, &nodes(r#"[{"spans": [{"text": "x"}]}, {}]"#)).is_empty());
        assert!(warned(empty, &nodes(r#"[{}, {"content": ""}]"#)).is_empty());
    }
}
