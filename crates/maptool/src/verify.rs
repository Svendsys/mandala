// SPDX-License-Identifier: MPL-2.0

//! `verify`'s warnings: what the loader accepts but is probably a
//! mistake.
//!
//! The hard failures are the loader's alone. `verify` loads the map with
//! [`file::read`](baumhard::document::format::file::read), and a refusal
//! is printed as the loader words it, with its JSON path; nothing here
//! judges what the loader judges. Once a map has loaded, [`warnings`]
//! looks for four things, as #253 lists them and its amendment E1.1
//! (issue comment 6870828150) rules:
//!
//! The edits read are the map's own, its `edits`. The App and User
//! tiers (`assets/edits.json`, the user's `edits.json`; `format/edits.md`
//! "Where edits come from") also run on a map, and they are outside what
//! a map file can be judged on: `verify` does not read them, and each
//! warning below says "in the map" for that reason.
//!
//! - **A verb nothing refers to.** A verb is referred to by a node's `on`
//!   (`click`, `hover`), by a span's `link` (`{"verb": …}`), and by an
//!   edit in the map that names it — a node's `set_click` or
//!   `set_hover`, a span's `set_link` to `{"verb": …}`, or a filter's
//!   `on_click` or `on_hover`, anywhere in it (its filter or its body)
//!   ([`EditDef::mentions`](baumhard::document::edit::EditDef::mentions))
//!   — and by nothing else in the format. An edit refers to what it names
//!   whether or not anything runs it: keybinds, the console and macros run
//!   an edit by id from outside the map, so a file cannot say an edit is
//!   never run. The issue's other half, a presentation nothing refers to,
//!   has no meaning until something can refer to a presentation; E1.1
//!   moves it to V1-D3 (#252), which adds the verb steps that will.
//! - **A node saved hidden that nothing can reveal.** What reveals a node
//!   is an edit that writes `set_hidden: false`, or a step of a
//!   presentation or a verb — and those are an id and a name until V1-D2
//!   and V1-D3 (#251, #252) give them steps. In general, which elements
//!   an edit reaches is decided when it runs — by the anchors it is run
//!   at, and by its filter and its mutator's keys against the map as it
//!   stands then, which other edits change — so `verify` does not narrow
//!   by reach: with an edit in the map that writes `set_hidden: false` no
//!   hidden node is reported, and without one, as E1.1 rules, every node
//!   saved with `"hidden": true` is, until D3 narrows it. The known case
//!   where this is conservative is an edit whose filter fixes its node: on
//!   `target: "node"`, a first test of `ident` `Equals` decides for every
//!   node, and no op changes an id, so that edit only ever reaches the one
//!   node. It is not narrowed for: a second rule for a rare case.
//! - **An empty frame**: one that resolves visible, through its own
//!   `visible`, the map's defaults and the built-in value, and holds no
//!   rings — it draws nothing. No edit can add a ring, so none can make it
//!   draw something.
//! - **A portal link whose ends both have no content**: each end has no
//!   spans and no `content` of its own, and the content it resolves to
//!   through the map's defaults is empty, so neither end draws a marker —
//!   unless an edit gives one. Which ends an edit reaches is, in general,
//!   decided when it runs, as above, so with an edit in the map that
//!   writes a portal end's `set_content` to a non-empty marker no such
//!   portal is reported. "Content" is judged two ways on purpose: an end
//!   saved with its own `content: ""` is not reported, since an explicit
//!   empty marker in the file is the author's choice; an edit counts only
//!   if the marker it gives draws, so its `set_content: ""` (or `null`,
//!   which inherits the defaults' empty content) does not. An edit cannot
//!   give an end spans or change the defaults.
//!
//! Each warning names its place as `grep` does: `#<id>` and the path
//! below the node, or a definition's path for a verb.

use baumhard::arena::NodeId;
use baumhard::document::edit::{Named, Shows};
use baumhard::document::element::{DocElement, DocKind, Triggers};
use baumhard::document::ids::Document;
use baumhard::document::resolve::{resolve_frame, resolve_portal_end, Content};
use baumhard::document::values::{LinkKind, LinkTarget, VerbId};

use crate::map::{ident, is, link, text_owners, Nodes};

/// Every warning `doc` earns, in the file's order: the nodes' in
/// pre-order — each node's own, then its frame's, then its links' — then
/// the verbs', in the order `verbs` lists them.
///
/// Cost: one walk of each edit, one scan of the verbs per verb an edit
/// names, one walk of the nodes and every element below them, and one
/// flag per verb.
pub fn warnings(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    let definitions = doc.definitions();
    let mut referenced = vec![false; definitions.verbs.len()];
    let (mut reveals, mut marks_portal_ends) = (false, false);
    for edit in &definitions.edits {
        let Shows {
            reveals: reveal,
            marks_portal_ends: marks,
        } = edit.mentions(|named| match named {
            Named::Verb(name) => {
                if let Some(n) = definitions.verbs.iter().position(|verb| verb.id == name) {
                    referenced[n] = true;
                }
            }
        });
        reveals |= reveal;
        marks_portal_ends |= marks;
    }
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
        // Every trigger, by name: one added to `Triggers` fails to compile
        // here instead of leaving its verbs unreferenced.
        let Triggers { click, hover } = data.on;
        click.into_iter().chain(hover).for_each(&mut refer);
        if data.hidden && !reveals {
            out.push(format!(
                "#{id}: hidden, and no edit in the map, verb or presentation can reveal it"
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
                if link.kind == LinkKind::Portal
                    && !marks_portal_ends
                    && doc.children(child).all(|end| empty_end(doc, end))
                {
                    out.push(format!(
                        "#{id} links[{links}]: a portal whose ends both have no content, and no edit in the map gives one"
                    ));
                }
                links += 1;
            }
        }
    }
    for (n, verb) in definitions.verbs.iter().enumerate() {
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
            ["#4: hidden, and no edit in the map, verb or presentation can reveal it"]
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
            ["#1 links[1]: a portal whose ends both have no content, and no edit in the map gives one"]
        );
        assert!(warned("", &nodes("[{}, {}]")).is_empty());
        assert!(warned(empty, &nodes(r#"[{"spans": [{"text": "x"}]}, {}]"#)).is_empty());
        assert!(warned(empty, &nodes(r#"[{}, {"content": ""}]"#)).is_empty());
    }

    /// The map the ruling on #250 answers: one verb, `go`, which only an
    /// edit's `set_click` names. It verifies with no warning.
    ///
    /// Control: not reading the edits (dropping the `definitions.edits`
    /// loop) brings back `verbs[0] "go": defined, and nothing refers to
    /// it`.
    #[test]
    fn test_verify_counts_a_verb_an_edit_names() {
        let header = r#""verbs": [{"id": "go", "name": "Go"}],
            "edits": [{"id": "c", "name": "C", "reach": "self", "target": "node",
                       "mutations": [{"node": {"set_click": "go"}}]}],"#;
        assert_eq!(
            warned(header, r#"{"id": 1, "position": [0, 0]}"#),
            Vec::<String>::new()
        );
    }

    /// The map's verbs `go` and `stay`, and `edit` as its one edit.
    fn edit_naming(edit: &str) -> String {
        format!(
            r#""verbs": [{{"id": "go", "name": "Go"}}, {{"id": "stay", "name": "Stay"}}],
               "edits": [{{"id": "e", "name": "E", "reach": "self", {edit}}}],"#
        )
    }

    /// Each way an edit names a verb, each where it is written: a node's
    /// `set_click` in `mutations`; its `set_hover` in a macro below a
    /// `void` root; a span's `set_link` to `{"verb": …}`; a filter's
    /// `on_click` in the edit's `filter`; and an `on_hover` in a
    /// `repeat_while` test. Each refers to `go` and to nothing else:
    /// `stay`, which no edit names, is still reported, so a named verb is
    /// counted alone and not every verb once an edit names one.
    ///
    /// Controls, each run and each failing only its own case with the
    /// `verbs[0] "go"` warning: dropping `SetClick` from its arm,
    /// `SetHover` from its arm, the `SetLink` verb arm, `OnClick` from its
    /// arm, `OnHover` from its arm; not recursing into a mutator's
    /// children fails the `set_hover` case; not reading a `repeat_while`
    /// test fails the `on_hover` case. Marking every verb referenced when
    /// any edit names one fails every case with `stay`'s warning missing.
    #[test]
    fn test_verify_counts_each_way_an_edit_names_a_verb() {
        let node = r#"{"id": 1, "position": [0, 0], "sections": [{"spans": [{"text": "a"}]}]}"#;
        let stay = [r#"verbs[1] "stay": defined, and nothing refers to it"#];
        let folds = r#"{"node": {"set_folded": true}}"#;
        let cases = [
            (
                "set_click",
                r#""target": "node", "mutations": [{"node": {"set_click": "go"}}]"#.to_string(),
            ),
            (
                "set_hover",
                r#""target": "node", "mutator": {"void": {"kind": "node", "channel": 0, "children": [
                     {"macro": {"kind": "node", "channel": 0, "mutations": [{"node": {"set_hover": "go"}}]}}]}}"#
                    .to_string(),
            ),
            (
                "set_link",
                r#""target": "spans", "mutations": [{"span": {"set_link": {"verb": "go"}}}]"#.to_string(),
            ),
            (
                "on_click",
                format!(
                    r#""target": "node", "filter": {{"fields": [[{{"on_click": "go"}}, {{"Equals": false}}]]}},
                       "mutations": [{folds}]"#
                ),
            ),
            (
                "on_hover",
                format!(
                    r#""target": "node", "mutator": {{"instruction": {{"kind": "node", "channel": 0,
                         "instruction": {{"repeat_while": {{"fields": [[{{"on_hover": "go"}}, {{"Equals": false}}]]}}}},
                         "children": [{{"macro": {{"kind": "node", "channel": 0, "mutations": [{folds}]}}}}]}}}}"#
                ),
            ),
        ];
        let wrong: Vec<_> = cases
            .iter()
            .map(|(way, edit)| (*way, warned(&edit_naming(edit), node)))
            .filter(|(_, warnings)| warnings != &stay)
            .collect();
        assert_eq!(wrong, [], "the cases whose warnings are not `stay`'s alone");
    }

    /// A map with an edit that writes `set_hidden: false` — here in a
    /// mutator's macro — reports no hidden node. One that writes only
    /// `set_hidden: true` reveals nothing, and the node is reported.
    ///
    /// Control: dropping the reveal (`self.shows.reveals |= !hidden` made a
    /// no-op) fails the first assertion with node 4's warning; setting it
    /// on any `set_hidden` fails the second.
    #[test]
    fn test_verify_counts_an_edit_that_reveals_a_node() {
        let edit = |hidden: bool| {
            format!(
                r#""edits": [{{"id": "e", "name": "E", "reach": "self", "target": "node",
                    "mutator": {{"macro": {{"kind": "node", "channel": 0, "mutations": [{{"node": {{"set_hidden": {hidden}}}}}]}}}}}}],"#
            )
        };
        let node = r#"{"id": 4, "position": [0, 0], "hidden": true}"#;
        assert_eq!(warned(&edit(false), node), Vec::<String>::new());
        assert_eq!(
            warned(&edit(true), node),
            ["#4: hidden, and no edit in the map, verb or presentation can reveal it"]
        );
    }

    /// A map with an edit that writes a portal end's `set_content` to a
    /// marker reports no portal whose ends have none; one that writes it
    /// empty, or to `null` (which inherits the defaults' empty content),
    /// gives no marker, and the portal is reported.
    ///
    /// Control: dropping the `SetContent` arm fails the first assertion;
    /// dropping its non-empty guard fails the second; counting `null` as a
    /// marker fails the third. Not recursing into a mutator's children
    /// also fails the first, since the portal end is reached from a
    /// `void` link root.
    #[test]
    fn test_verify_counts_an_edit_that_marks_a_portal_end() {
        let header = |content: &str| {
            format!(
                r#""defaults": {{"portal_end": {{"content": ""}}}},
                   "edits": [{{"id": "e", "name": "E", "reach": "self", "target": "links",
                     "mutator": {{"void": {{"kind": "link", "channel": 0, "children": [
                       {{"macro": {{"kind": "portal_end", "channel": 0, "mutations": [{{"portal_end": {{"set_content": {content}}}}}]}}}}]}}}}}}],"#
            )
        };
        let nodes = r#"{"id": 1, "position": [0, 0], "links": [{"to": 2, "kind": "portal"}]},
                       {"id": 2, "position": [0, 0]}"#;
        let warning =
            ["#1 links[0]: a portal whose ends both have no content, and no edit in the map gives one"];
        assert_eq!(warned(&header(r#""◆""#), nodes), Vec::<String>::new());
        assert_eq!(warned(&header(r#""""#), nodes), warning);
        assert_eq!(warned(&header("null"), nodes), warning);
    }

    /// What one edit counts stays counted when later edits count
    /// nothing: the first edit reveals and names `go`, the second marks a
    /// portal end, and the third does neither. A hidden node, an empty
    /// portal and `go` are then all unreported.
    ///
    /// Controls, each run: `reveals |= reveal` made `reveals = reveal`
    /// reports node 1; `marks_portal_ends |= marks` made `=` reports the
    /// portal; clearing `referenced` at the start of each edit reports
    /// `go`.
    #[test]
    fn test_verify_keeps_what_an_earlier_edit_counts() {
        let header = r#""defaults": {"portal_end": {"content": ""}},
            "verbs": [{"id": "go", "name": "Go"}],
            "edits": [
              {"id": "a", "name": "A", "reach": "self", "target": "node",
               "mutations": [{"node": {"set_hidden": false}}, {"node": {"set_click": "go"}}]},
              {"id": "b", "name": "B", "reach": "self", "target": "links",
               "mutator": {"void": {"kind": "link", "channel": 0, "children": [
                 {"macro": {"kind": "portal_end", "channel": 0, "mutations": [{"portal_end": {"set_content": "◆"}}]}}]}}},
              {"id": "c", "name": "C", "reach": "self", "target": "node",
               "mutations": [{"node": {"set_folded": true}}]}
            ],"#;
        let nodes = r#"{"id": 1, "position": [0, 0], "hidden": true, "links": [{"to": 2, "kind": "portal"}]},
                       {"id": 2, "position": [0, 0]}"#;
        assert_eq!(warned(header, nodes), Vec::<String>::new());
    }
}
