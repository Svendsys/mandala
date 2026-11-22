// SPDX-License-Identifier: MPL-2.0

//! `verify`'s warnings: what the loader accepts but is probably a
//! mistake.
//!
//! The hard failures are the loader's alone. `verify` loads the map with
//! [`file::read`](baumhard::document::format::file::read), and a refusal
//! is printed as the loader words it, with its JSON path; nothing here
//! judges what the loader judges. Once a map has loaded, [`warnings`]
//! looks for six things: five as #253 lists them, its amendment E1.1
//! (issue comment 6870828150) rules and V1-D3 (#252) completes, and a
//! sixth that V1-B40 (#359) adds:
//!
//! - **A verb nothing refers to.** A verb is referred to by a node's `on`
//!   (`click`, `hover`), by a span's `link` (`{"verb": …}`), and by an
//!   edit or a presentation in the map that names it — a node's
//!   `set_click` or `set_hover`, a span's `set_link` to `{"verb": …}`, or
//!   a filter's `on_click` or `on_hover`, anywhere in it (its filter or
//!   its body)
//!   ([`EditDef::mentions`](baumhard::document::edit::EditDef::mentions),
//!   [`PresentationDef::mentions`](baumhard::document::presentation::PresentationDef::mentions))
//!   — by another verb's `verb` step (V1-D3), and by nothing else in the
//!   format. An edit refers to what it names whether or not anything runs
//!   it: keybinds, the console and macros run an edit by id from outside
//!   the map, so a file cannot say an edit is never run; the console
//!   starts a presentation the same way.
//! - **A presentation nothing refers to** (E1.1 item 1, moved to V1-D3):
//!   what refers to one of the map's presentations is a verb's `present`
//!   or `stop` step, and nothing else in the format. A presentation no
//!   step names can still be started from the console by its id, as an
//!   unreferenced verb can be run from there, and is reported all the
//!   same: the map itself never plays it.
//! - **A node saved hidden that nothing can reveal.** What reveals a node
//!   is an edit that writes `set_hidden: false`, a presentation that
//!   plays on nodes and writes it (V1-D2, #251: the node is shown for as
//!   long as it plays), or a step of a verb — and a verb's steps (V1-D3,
//!   #252) reveal a node only by running one of the map's own edits or
//!   presentations (§11.3), which this already counts, so a verb narrows
//!   nothing it does not: a map whose edits and presentations reveal
//!   nothing has no verb that can either. In general, which elements
//!   an edit reaches is decided when it runs — by the anchors it is run
//!   at, and by its filter and its mutator's keys against the map as it
//!   stands then, which other edits change — and a presentation's by the
//!   nodes it is started on, so `verify` does not narrow by reach: with
//!   an edit or a presentation in the map that writes `set_hidden: false`
//!   no hidden node is reported, and without one every node saved with
//!   `"hidden": true` is. Narrowing by reach is not sound: the console
//!   starts any of the map's presentations, and runs any of its edits, on
//!   any node, whatever the verbs' targets name. The known case
//!   where this is conservative is an edit whose filter fixes its node: on
//!   `target: "node"`, a first test of `ident` `Equals` decides for every
//!   node, and no op changes an id, so that edit only ever reaches the one
//!   node. `verify` does not narrow for it: that would be a second rule
//!   for a rare case.
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
//!   give an end spans or change the defaults. A presentation cannot
//!   mark a portal end at all: it has no mutator, and none of its
//!   targets is a portal end.
//! - **A name a map's own edit or presentation writes that the map does
//!   not define**
//!   (V1-B40, #359): a verb, a theme variable (`var(name)`, in any color
//!   an edit writes) or a palette. App and User edits resolve their
//!   names when they run (`format/edits.md`, "Names are resolved when
//!   the edit runs"), so the loader cannot judge one against a map, and
//!   a map whose own edit names something it lacks loads and then has
//!   every run of that edit refused. The names come from the same walk
//!   as everything above
//!   ([`EditDef::mentions`](baumhard::document::edit::EditDef::mentions),
//!   in which a step that can name one fails to compile until it is
//!   counted), and "defined" is
//!   [`Mention::is_defined_in`](baumhard::document::edit::Mention::is_defined_in),
//!   the probes of the document a run resolves the name by, so this
//!   warns exactly where a run would be refused for a name and nowhere
//!   else. It reports every such name an
//!   edit writes, in the order the edit writes them, in the words of the
//!   refusal (`EditError::UnknownName`), not the first alone as a run
//!   stops at. A map's verbs, theme variables and palettes are the
//!   file's own, and no operation adds, removes or renames one, so the
//!   file decides. What it cannot say is whether a run is ever asked
//!   for, and that is not a defect of the name: keybinds, the console
//!   and macros run an edit by id from outside the map. A map's own
//!   presentation is judged the same way: it writes the same names, they
//!   are resolved when it starts, and its warning is the words of that
//!   refusal (`PresentError::UnknownName`). Each warning leads with
//!   where the name is written, `edits[n]` or `presentations[n]` and the
//!   JSON path below it that the loader's own paths use
//!   (`mutations[1].node.text.set_color`,
//!   `filter.fields[0][0].background`), so that a name written in
//!   several places is several warnings that can be told apart, and the
//!   field it names is the key the file writes there. Fonts are not
//!   names a map defines; the loader's font rules decide them
//!   (`format/fonts.md`).
//!
//! The edits and presentations read are the map's own, its `edits` and
//! its `presentations`. The App and User tiers (`assets/edits.json`,
//! the user's `edits.json`; `format/edits.md` "Where edits come from";
//! and the presentations' likewise) also run on a map, and they are
//! outside what a map file can be judged on: `verify` does not read
//! them, and the hidden-node and portal warnings say "in the map" for
//! that reason.
//!
//! Each warning names its place as `grep` does: `#<id>` and the path
//! below the node, or a definition's path for a verb.

use baumhard::arena::NodeId;
use baumhard::document::edit::{Mention, Named, Shows};
use baumhard::document::element::{DocElement, DocKind, Triggers};
use baumhard::document::ids::Document;
use baumhard::document::resolve::{resolve_frame, resolve_portal_end, Content};
use baumhard::document::values::{LinkKind, LinkTarget, VerbId};
use baumhard::document::verb::Step;

use crate::map::{ident, is, link, text_owners, Nodes};

/// Every warning `doc` earns, in this order: the nodes' in pre-order —
/// each node's own, then its frame's, then its links' — then the names
/// the edits write that the map does not define, in the order `edits`
/// lists them (each edit's in the order the walk reaches them: its
/// filter, then its mutations or its mutator, in order), then the
/// presentations' names likewise, then the presentations no verb names,
/// in the order `presentations` lists them, then the verbs', in the
/// order `verbs` lists them. It is not the file's order, which writes
/// the nodes last.
///
/// Cost: one walk of each edit and each presentation, one scan of the
/// verbs per verb one of them names, one walk of the nodes and every
/// element below them, and one flag per verb.
pub fn warnings(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    let definitions = doc.definitions();
    let mut referenced = vec![false; definitions.verbs.len()];
    let (mut reveals, mut marks_portal_ends) = (false, false);
    // What each definition names and can show, an edit or a
    // presentation: every field of `Shows` by name, so one added fails
    // to compile here instead of going uncounted.
    let mut name = |mention: &Mention| {
        if let Named::Verb = mention.kind {
            if let Some(n) = definitions.verbs.iter().position(|verb| verb.id == mention.name) {
                referenced[n] = true;
            }
        }
    };
    let mut shown = Vec::new();
    // A map's own edit names things the map must define when it runs
    // (`format/edits.md`, "Names are resolved when the edit runs"): each
    // name it writes that the map does not is a warning, from the same
    // walk, and by the same probes of the document, a run resolves by.
    let mut undefined = Vec::new();
    for (n, edit) in definitions.edits.iter().enumerate() {
        shown.push(edit.mentions(|mention| {
            name(&mention);
            if !mention.is_defined_in(doc) {
                undefined.push(format!(
                    "edits[{n}].{}: {}",
                    mention.path,
                    mention.refusal(edit.id.as_str())
                ));
            }
        }));
    }
    // A map's own presentation is judged the same way: its start is
    // refused for the same names, by the same probes.
    for (n, presentation) in definitions.presentations.iter().enumerate() {
        shown.push(presentation.mentions(|mention| {
            name(&mention);
            if !mention.is_defined_in(doc) {
                undefined.push(format!(
                    "presentations[{n}].{}: {}",
                    mention.path,
                    mention.presentation_refusal(presentation.id.as_str())
                ));
            }
        }));
    }
    for Shows {
        reveals: reveal,
        marks_portal_ends: marks,
    } in shown
    {
        reveals |= reveal;
        marks_portal_ends |= marks;
    }
    // What the verbs' steps name: the presentations they start and
    // stop, and the verbs they run. Every step kind by name, so one added
    // fails to compile here instead of going uncounted.
    let mut played = vec![false; definitions.presentations.len()];
    for verb in &definitions.verbs {
        for step in &verb.steps {
            match step {
                Step::Present { presentation, .. } | Step::Stop { presentation, .. } => {
                    if let Some(flag) = played.get_mut(usize::from(presentation.0)) {
                        *flag = true;
                    }
                }
                Step::Verb { verb } => {
                    if let Some(flag) = referenced.get_mut(usize::from(verb.0)) {
                        *flag = true;
                    }
                }
                Step::Edit { .. } | Step::Wait { .. } => {}
            }
        }
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
    out.extend(undefined);
    for (n, presentation) in definitions.presentations.iter().enumerate() {
        if !played[n] {
            out.push(format!(
                "presentations[{n}] {:?}: defined, and no verb starts or stops it",
                presentation.id
            ));
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

    /// [`warned`], without the warnings of presentations no verb names:
    /// for the cases about what a presentation's body counts, whose
    /// presentations no verb plays.
    fn warned_but_unplayed(header: &str, nodes: &str) -> Vec<String> {
        warned(header, nodes)
            .into_iter()
            .filter(|warning| !warning.starts_with("presentations["))
            .collect()
    }

    /// A verb nothing refers to is reported by its place in `verbs`; one
    /// a node's `on` or a span's `link` names is not.
    ///
    /// Control: not calling `refer` for the triggers fails the `on` case,
    /// and not calling it for spans fails the span case — each with the
    /// `verbs[0]` warning the case expects to be absent.
    #[test]
    fn test_verify_warns_of_a_verb_nothing_refers_to() {
        let verbs = r#""verbs": [{"id": "go", "name": "Go", "steps": []}, {"id": "stay", "name": "Stay", "steps": []}],"#;
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

    /// A presentation no verb step names is reported by its place in
    /// `presentations`; one a `present` or a `stop` step names is not, and
    /// a verb only another verb's `verb` step names is referred to.
    ///
    /// Controls, each seen red: not marking `Stop` steps reports `p`;
    /// not counting `Verb` steps reports `inner`.
    #[test]
    fn test_verify_warns_of_a_presentation_no_verb_names() {
        let header = r#""presentations": [{"id": "p", "name": "P", "play": "hold", "target": "node", "mutations": []},
                {"id": "q", "name": "Q", "play": "hold", "target": "node", "mutations": []},
                {"id": "r", "name": "R", "play": "hold", "target": "node", "mutations": []}],
            "verbs": [{"id": "inner", "name": "I", "steps": [{"stop": "p", "on": "selection"}]},
                {"id": "outer", "name": "O", "steps": [{"present": "q", "on": "selection"}, {"verb": "inner"}]}],"#;
        let node = r#"{"id": 1, "position": [0, 0], "on": {"click": "outer"}}"#;
        assert_eq!(
            warned(header, node),
            [r#"presentations[2] "r": defined, and no verb starts or stops it"#]
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
    /// loop) brings back `verbs[0] "go": defined, and nothing refers
    /// to it`.
    #[test]
    fn test_verify_counts_a_verb_an_edit_names() {
        let header = r#""verbs": [{"id": "go", "name": "Go", "steps": []}],
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
            r#""verbs": [{{"id": "go", "name": "Go", "steps": []}}, {{"id": "stay", "name": "Stay", "steps": []}}],
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

    /// A presentation in the map refers to the verbs it names, as an
    /// edit does — in its mutations and in its filter — and to nothing
    /// else: `stay`, which nothing names, is still reported.
    ///
    /// Control: not reading the presentations (dropping the
    /// `definitions.presentations` loop) reports `go` in both cases.
    #[test]
    fn test_verify_counts_a_verb_a_presentation_names() {
        let verbs = r#""verbs": [{"id": "go", "name": "Go", "steps": []}, {"id": "stay", "name": "Stay", "steps": []}],"#;
        let stay = [r#"verbs[1] "stay": defined, and nothing refers to it"#];
        let node = r#"{"id": 1, "position": [0, 0]}"#;
        let by_mutation = format!(
            r#"{verbs} "presentations": [{{"id": "p", "name": "P", "play": "hold", "target": "node",
                "mutations": [{{"node": {{"set_click": "go"}}}}]}}],"#
        );
        assert_eq!(warned_but_unplayed(&by_mutation, node), stay);
        let by_filter = format!(
            r#"{verbs} "presentations": [{{"id": "p", "name": "P", "play": "hold", "target": "node",
                "filter": {{"fields": [[{{"on_hover": "go"}}, {{"Equals": false}}]]}},
                "mutations": [{{"node": {{"set_opacity": 0.5}}}}]}}],"#
        );
        assert_eq!(warned_but_unplayed(&by_filter, node), stay);
    }

    /// A presentation that plays on nodes and writes `set_hidden: false`
    /// can show a node saved hidden, so none is reported; one that
    /// writes `set_hidden: true` reveals nothing, and neither does a
    /// glyph-layer presentation, which writes runs that are drawn.
    ///
    /// Controls: counting any `set_hidden` as a reveal fails the second
    /// assertion; not reading the presentations fails the first.
    #[test]
    fn test_verify_counts_a_presentation_that_reveals_a_node() {
        let presentation = |hidden: bool| {
            format!(
                r#""presentations": [{{"id": "p", "name": "P", "play": "hold", "target": "node",
                    "mutations": [{{"node": {{"set_hidden": {hidden}}}}}]}}],"#
            )
        };
        let node = r#"{"id": 4, "position": [0, 0], "hidden": true}"#;
        let warning = ["#4: hidden, and no edit in the map, verb or presentation can reveal it"];
        assert_eq!(
            warned_but_unplayed(&presentation(false), node),
            Vec::<String>::new()
        );
        assert_eq!(warned_but_unplayed(&presentation(true), node), warning);
        let glyph = r#""presentations": [{"id": "g", "name": "G", "play": "hold", "layer": "glyph",
            "target": "sections", "glyph": [{"run_opacity": 1.0}]}],"#;
        assert_eq!(warned_but_unplayed(glyph, node), warning);
    }

    /// What one edit counts stays counted when later edits count
    /// nothing: the first edit reveals and names `go`, the second marks a
    /// portal end, and the third does neither. A hidden node, an empty
    /// portal and `go` are then all unreported.
    ///
    /// Controls, each run: `reveals |= reveal` made `reveals = reveal`
    /// reports node 1; `marks_portal_ends |= marks` made `=` reports the
    /// portal; clearing `referenced` at the start of each edit
    /// reports `go`.
    #[test]
    fn test_verify_keeps_what_an_earlier_edit_counts() {
        let header = r#""defaults": {"portal_end": {"content": ""}},
            "verbs": [{"id": "go", "name": "Go", "steps": []}],
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

    /// The map `defines` (a header fragment: its verbs, theme or palettes,
    /// or nothing) and its one edit `e`, written as `body`.
    fn with_edit(defines: &str, body: &str) -> String {
        format!(r#"{defines} "edits": [{{"id": "e", "name": "E", "reach": "self", {body}}}],"#)
    }

    const NODE: &str = r#"{"id": 1, "position": [0, 0], "sections": [{"spans": [{"text": "a"}]}]}"#;
    const THEME: &str = r##""theme": {"active": "d", "variants": {"d": {"accent": "#ff0000"}}},"##;
    const PALETTE: &str = r##""palettes": {"warm": [{"frame": "#ff0000", "text": "#ff0000"}]},"##;

    /// A map's own edit that names a verb the map does not define is
    /// reported, naming the edit, the field and the name as the refusal of
    /// a run words them (`format/edits.md`); one naming a verb the map
    /// defines is not.
    ///
    /// Controls, each run: `Mention::is_defined_in` answering `true` for a
    /// verb fails the first assertion with no warning; answering `false`
    /// fails the second with the warning of a name the map defines.
    #[test]
    fn test_verify_warns_of_a_verb_an_edit_names_that_the_map_lacks() {
        let body = r#""target": "node", "mutations": [{"node": {"set_click": "nowhere"}}]"#;
        assert_eq!(
            warned(&with_edit("", body), NODE),
            ["edits[0].mutations[0].node.set_click: edit `e`: `set_click` names verb `nowhere`, which this map does not define"]
        );
        let defined = r#""verbs": [{"id": "nowhere", "name": "N", "steps": []}],"#;
        assert_eq!(warned(&with_edit(defined, body), NODE), Vec::<String>::new());
    }

    /// The same for a theme variable: `var(accent)` in a color, on a map
    /// whose theme has no `accent`.
    ///
    /// Controls, each run: `Walk::color` not reporting a `Var` fails the
    /// first assertion with no warning; `is_defined_in` answering `false`
    /// for a variable fails the second.
    #[test]
    fn test_verify_warns_of_a_theme_variable_an_edit_names_that_the_map_lacks() {
        let body = r#""target": "spans", "mutations": [{"span": {"text": {"set_color": "var(accent)"}}}]"#;
        assert_eq!(
            warned(&with_edit("", body), NODE),
            ["edits[0].mutations[0].span.text.set_color: edit `e`: `set_color` names theme variable `accent`, which this map does not define"]
        );
        assert_eq!(warned(&with_edit(THEME, body), NODE), Vec::<String>::new());
    }

    /// The same for a palette: a node's `set_palette`, on a map with no
    /// palette `warm`.
    ///
    /// Controls, each run: the `set_palette` arm not reporting its name
    /// fails the first assertion with no warning; `is_defined_in`
    /// answering `false` for a palette fails the second.
    #[test]
    fn test_verify_warns_of_a_palette_an_edit_names_that_the_map_lacks() {
        let body = r#""target": "node", "mutations": [{"node": {"set_palette": "warm"}}]"#;
        assert_eq!(
            warned(&with_edit("", body), NODE),
            ["edits[0].mutations[0].node.set_palette: edit `e`: `set_palette` names palette `warm`, which this map does not define"]
        );
        assert_eq!(warned(&with_edit(PALETTE, body), NODE), Vec::<String>::new());
    }

    /// Every name an edit writes that the map lacks is reported, not the
    /// first alone, in the order the edit writes them and under the
    /// edit's own place in `edits`; a name the map defines is not. An edit
    /// naming nothing the map lacks, and a handler, which names nothing,
    /// earn no warning.
    ///
    /// Controls, each run: reporting only the first undefined name fails
    /// with the second and third warnings missing; reporting every name,
    /// defined or not, adds `go`'s.
    #[test]
    fn test_verify_warns_of_every_name_an_edit_lacks_in_the_order_it_writes_them() {
        let header = r#""verbs": [{"id": "go", "name": "Go", "steps": []}],
               "edits": [
                 {"id": "a", "name": "A", "reach": "self", "target": "node", "mutations": [{"node": {"set_folded": true}}]},
                 {"id": "b", "name": "B", "reach": "self", "target": "node", "mutations": [
                   {"node": {"set_click": "go"}}, {"node": {"set_hover": "nowhere"}},
                   {"node": {"set_palette": "warm"}}, {"node": {"text": {"set_color": "var(accent)"}}}]},
                 {"id": "c", "name": "C", "reach": "self", "target": "node", "handler": "flower"}
               ],"#;
        assert_eq!(
            warned(header, NODE),
            [
                "edits[1].mutations[1].node.set_hover: edit `b`: `set_hover` names verb `nowhere`, which this map does not define",
                "edits[1].mutations[2].node.set_palette: edit `b`: `set_palette` names palette `warm`, which this map does not define",
                "edits[1].mutations[3].node.text.set_color: edit `b`: `set_color` names theme variable `accent`, which this map does not define",
            ]
        );
    }

    /// The map `defines` (a header fragment, or nothing) and its one
    /// presentation `p`, written as `body`, which a verb starts so that
    /// the only warning a case can earn is the name's.
    fn with_presentation(defines: &str, body: &str) -> String {
        format!(
            r#"{defines} "presentations": [{{"id": "p", "name": "P", "play": "hold", "target": "node", {body}}}],"#
        )
    }

    /// A map's own presentation that names a verb, a theme variable or a
    /// palette the map does not define is reported, in the words of the
    /// refusal of its start, under `presentations[n]`; one naming what
    /// the map defines is not.
    ///
    /// Controls, each run: not judging presentations (the check dropped
    /// from the presentations' loop) fails the three first assertions
    /// with no warning; `is_defined_in` answering `false` fails the
    /// second ones.
    #[test]
    fn test_verify_warns_of_a_name_a_presentation_names_that_the_map_lacks() {
        let verb = r#""mutations": [{"node": {"set_click": "nowhere"}}]"#;
        let variable = r#""mutations": [{"node": {"text": {"set_color": "var(accent)"}}}]"#;
        let palette = r#""filter": {"fields": [[{"palette": "warm"}, {"Equals": false}]]},
                         "mutations": [{"node": {"set_folded": true}}]"#;
        let unplayed = r#"presentations[0] "p": defined, and no verb starts or stops it"#;
        let verbs = r#""verbs": [{"id": "nowhere", "name": "N", "steps": []}],"#;
        for (body, defines, warning) in [
            (
                verb,
                verbs,
                "presentations[0].mutations[0].node.set_click: presentation `p`: `set_click` names verb `nowhere`, which this map does not define",
            ),
            (
                variable,
                THEME,
                "presentations[0].mutations[0].node.text.set_color: presentation `p`: `set_color` names theme variable `accent`, which this map does not define",
            ),
            (
                palette,
                PALETTE,
                "presentations[0].filter.fields[0][0].palette: presentation `p`: `palette` names palette `warm`, which this map does not define",
            ),
        ] {
            assert_eq!(warned(&with_presentation("", body), NODE), [warning, unplayed]);
            // The map defining the name: nothing to report.
            let defined = warned_but_unplayed(&with_presentation(defines, body), NODE);
            assert_eq!(defined, Vec::<String>::new(), "{defines}");
        }
    }

    /// One name written in three places of one edit is three warnings
    /// that can be told apart: each leads with where the file writes it,
    /// and names the key it is written at (`set_color`, `background`),
    /// not the `color` an op's value is called in general. This is the
    /// map that earned three identical lines.
    ///
    /// Controls, each run: the mutation's index left out of the path
    /// gives two warnings the same place; the filter not walked loses
    /// the first warning; a filter's `background` called `color` fails
    /// the key.
    #[test]
    fn test_verify_tells_apart_the_places_one_name_is_written() {
        let body = r#""target": "node",
            "filter": {"fields": [[{"background": "var(accent)"}, {"Equals": false}]]},
            "mutations": [{"node": {"text": {"set_color": "var(accent)"}}},
                          {"node": {"set_folded": true}},
                          {"node": {"text": {"set_color": "var(accent)"}}}]"#;
        assert_eq!(
            warned(&with_edit("", body), NODE),
            [
                "edits[0].filter.fields[0][0].background: edit `e`: `background` names theme variable `accent`, which this map does not define",
                "edits[0].mutations[0].node.text.set_color: edit `e`: `set_color` names theme variable `accent`, which this map does not define",
                "edits[0].mutations[2].node.text.set_color: edit `e`: `set_color` names theme variable `accent`, which this map does not define",
            ]
        );
    }
}
