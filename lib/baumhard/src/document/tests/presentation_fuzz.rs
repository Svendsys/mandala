// SPDX-License-Identifier: MPL-2.0

//! The presentation fuzz properties (design D29, amendment on #251;
//! TEST_CONVENTIONS §T14): presentations a map, a user or the app
//! defines are an untrusted-input surface, so any bytes read as a
//! presentations file, and presentations built from choices, hold to:
//!
//! - **no panic**, in the reader, a start, a frame at any clock reading,
//!   a stop, or a change made under what plays;
//! - **a refusal names its JSON path**: a presentation that does not
//!   read is refused under `presentations[i]`;
//! - **bounded time per frame**: a frame brings each instance to the
//!   clock's reading with arithmetic, however long ago the last frame
//!   was — a reading a million million milliseconds on is one frame's
//!   work, a loop's cycles in between are not walked — which the
//!   harness's per-input timeout checks;
//! - **bounded time per lifecycle**: an instance that is stopped is
//!   over once its directions have run — at most one way in and one way
//!   out more, each at most a delay and a duration of
//!   [`MAX_ANIMATE_MS`] — so [`LIFECYCLE_MS`] after a stop nothing
//!   plays;
//! - **a running presentation never changes what a save writes**:
//!   after the starts, at every frame, after a change under what plays
//!   and after the stops, the map saves to the bytes the same map saves
//!   to with nothing playing, its revision and history the same;
//! - **a change is the map's own** (ruling D2.1 §2 on #251): an edit
//!   built from the map itself and applied under what plays leaves the
//!   history journals, image for image, and the saved bytes that it
//!   leaves with nothing playing, and so do its undo and its redo;
//! - **a glyph-layer body is run mutations**: what it spells holds
//!   every property of authored run mutations
//!   ([`check_glyph_body`]).
//!
//! The fuzz targets (`fuzz/fuzz_targets/presentations_*.rs`) are thin
//! wrappers over [`check_presentations_json`] and
//! [`check_presentations_structured`]; the sweep in
//! `presentation_fuzz_tests` runs the generator on stable, in
//! `./test.sh`.

use crate::arena::NodeId;
use crate::document::bounds::MAX_ANIMATE_MS;
use crate::document::edit::{parse_edits, EditDef};
use crate::document::element::{DocElement, DocKind};
use crate::document::format::{load, save, LoadErrorKind};
use crate::document::ids::Document;
use crate::document::presentation::{parse_presentations, PresentationBody};
use crate::document::tests::edit_fuzz::{self, Boxes, FIELDS, MUTATIONS, TARGETS};
use crate::document::writer::ChangeSource;
use crate::edit_handlers::registry;
use crate::engine::journal::Touch;
use crate::projection::tests::presentation_glyphs::check_glyph_body;
use crate::util::tests::choices::Choices;

/// The map presentations play on: the format's fullest fixture.
const MAP: &str = include_str!("../format/fixtures/full.mandala.json");

/// How long after a stop every instance is over: a loop finishes the
/// cycle it is in — the rest of its way in, then its way out — and each
/// way is at most a delay and a duration of [`MAX_ANIMATE_MS`]. One
/// millisecond past that, so that the reading is after the last end
/// and not on it.
pub const LIFECYCLE_MS: f64 = 4.0 * MAX_ANIMATE_MS as f64 + 1.0;

/// The most definitions of one file that are played, one on top of
/// the other. A file may hold any number; the rest are read, and held
/// to the reader's properties alone.
pub const MAX_PLAYED: usize = 8;

/// Edits run under what plays, by a raw input: one that reads nothing
/// to build its change, one whose filter reads a font size, one whose
/// filter reads an opacity, and the two layouts, which read where the
/// nodes are.
const EDITS: &str = r##"{"edits":[
{"id":"nudge","name":"N","reach":"self","target":"node","mutations":[{"node":{"nudge":[50.0,0.0]}}]},
{"id":"big-text","name":"B","reach":"subtree","target":"spans",
 "filter":{"fields":[[{"font_size":{"pt":12.0}},{"GreaterThan":false}]]},
 "mutations":[{"span":{"text":{"set_color":"#102030"}}}]},
{"id":"opaque","name":"O","reach":"subtree","target":"node",
 "filter":{"fields":[[{"opacity":0.5},{"GreaterThan":false}]]},
 "mutations":[{"node":{"set_opacity":0.75}}]},
{"id":"flower","name":"F","reach":"self","target":"node","handler":"flower"},
{"id":"cascade","name":"C","reach":"self","target":"node","handler":"cascade"}
]}"##;

/// What one input played: how far the properties were exercised, for
/// the sweeps that hold them to having been.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Played {
    /// Definitions that started at least one instance.
    pub started: usize,
    /// Edits that changed the map under an instance.
    pub edited: usize,
}

/// Every property of `bytes` read as a presentations file.
pub fn check_presentations_json(bytes: &[u8]) -> Result<(), String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(());
    };
    check_presentations_text(text, true).map(|_| ())
}

/// [`check_presentations_json`] for a text, with what it played. The
/// edits run under what plays are built from the map itself when
/// `from_the_map` — as every funnel of the app builds a change — and
/// from the map as it is drawn otherwise: the control that shows the
/// differential property can fail.
pub fn check_presentations_text(text: &str, from_the_map: bool) -> Result<Played, String> {
    let edits = parse_edits(EDITS).map_err(|error| format!("the fixed edits read: {error}"))?;
    check_text(text, false, &edits, &[250.0, 1_000.0], from_the_map)
}

/// Every property of the presentations file `bytes` choose.
pub fn check_presentations_structured(bytes: &[u8]) -> Result<(), String> {
    check_presentations_generated(&mut Choices::new(bytes)).map(|_| ())
}

/// [`check_presentations_structured`] for choices from any source: the
/// file's properties, with frames at two readings the choices draw and
/// edits the choices build ([`edit_fuzz::generate`]) run under what
/// plays, and the same presentations read inside a map, its `nodes`
/// before or after them.
pub fn check_presentations_generated(choices: &mut Choices<'_>) -> Result<Played, String> {
    let file = generate(choices);
    let frames = [f64::from(choices.below(2_000)), f64::from(choices.below(200_000))];
    // The fixed edits, then the ones the choices build: a file of those
    // that does not read is the edits targets' to hold.
    let mut edits = parse_edits(EDITS).map_err(|error| format!("the fixed edits read: {error}"))?;
    edits.extend(parse_edits(&edit_fuzz::generate(choices)).unwrap_or_default());
    let nodes_first = choices.one_in(2);
    let played = check_text(&file, true, &edits, &frames, true)?;
    check_in_a_map(&file, nodes_first)?;
    Ok(played)
}

/// A map's `presentations` read as a presentations file's do: both
/// read, or both are refused with the same kind at the same path —
/// whatever comes before them in the map. `file` is one of
/// [`generate`]'s.
fn check_in_a_map(file: &str, nodes_first: bool) -> Result<(), String> {
    let list = file
        .strip_prefix(r#"{"presentations":"#)
        .and_then(|rest| rest.strip_suffix('}'))
        .ok_or("a generated file is `{\"presentations\":[…]}`")?;
    let nodes = r#""nodes":[{"id":1,"position":[0,0]}]"#;
    let map = if nodes_first {
        format!(r#"{{"format":1,"name":"t",{nodes},"presentations":{list}}}"#)
    } else {
        format!(r#"{{"format":1,"name":"t","presentations":{list},{nodes}}}"#)
    };
    match (parse_presentations(file), load(&map)) {
        (Ok(_), Ok(_)) => Ok(()),
        (Err(a), Err(b)) if a.kind == b.kind && a.path == b.path => Ok(()),
        (file_read, map_read) => Err(format!(
            "a presentations file and a map read the same presentations apart: {:?} and {:?}\n{map}",
            file_read.err(),
            map_read.err()
        )),
    }
}

/// One entry of a journal: the element, what happened to it, and its
/// image before and after.
type Image = (NodeId, Touch, Option<DocElement>, Option<DocElement>);

/// What a document's applied history holds, image for image.
fn journals(doc: &Document) -> Vec<Vec<Image>> {
    doc.applied_journals()
        .map(|journal| {
            journal
                .touched()
                .map(|(id, touch)| (id, touch, journal.before(id).cloned(), journal.after(id).cloned()))
                .collect()
        })
        .collect()
}

/// The two maps a run holds to each other: the one presentations play
/// on, and the same map with nothing playing, which every change is
/// made on as well.
struct Maps {
    performed: Document,
    plain: Document,
    /// What `plain` saves to as it stands.
    saved: String,
}

impl Maps {
    /// Two copies of `map`.
    fn of(mut map: Document) -> Result<Self, String> {
        let saved = map
            .save_json()
            .map_err(|error| format!("the map saves: {error}"))?;
        Ok(Maps {
            performed: map.clone(),
            plain: map,
            saved,
        })
    }

    /// `plain`, and `performed`: the same map with presentations
    /// playing on it.
    fn beside(mut plain: Document, performed: Document) -> Result<Self, String> {
        let saved = plain
            .save_json()
            .map_err(|error| format!("the map saves: {error}"))?;
        Ok(Maps {
            performed,
            plain,
            saved,
        })
    }

    /// What plays has not reached what a reader holding the document
    /// shared saves: the bytes `plain` saves to.
    fn same_save(&self, when: &str) -> Result<(), String> {
        let shared =
            save(&self.performed).map_err(|error| format!("{when}: the map no longer saves: {error}"))?;
        if shared != self.saved {
            return Err(format!("{when}: the map saves to other bytes under what plays"));
        }
        Ok(())
    }

    /// [`Self::same_save`], and the writer's own save, the revision, the
    /// unsaved-changes flag and how many entries the history holds.
    fn same_map(&mut self, when: &str) -> Result<(), String> {
        self.same_save(when)?;
        let own = self
            .performed
            .save_json()
            .map_err(|error| format!("{when}: the map no longer saves: {error}"))?;
        if own != self.saved {
            return Err(format!("{when}: the map saves to other bytes under what plays"));
        }
        if self.performed.revision() != self.plain.revision()
            || self.performed.is_dirty() != self.plain.is_dirty()
            || self.performed.history().count() != self.plain.history().count()
        {
            return Err(format!(
                "{when}: the revision, the unsaved-changes flag or the history's length is another under what \
                 plays"
            ));
        }
        Ok(())
    }

    /// `plain` changed: [`Self::same_map`] against what it saves to now,
    /// and the history image for image — what a change made under what
    /// plays is held to.
    fn same_change(&mut self, when: &str) -> Result<(), String> {
        self.saved = self
            .plain
            .save_json()
            .map_err(|error| format!("{when}: the map no longer saves: {error}"))?;
        self.same_map(when)?;
        if journals(&self.performed) != journals(&self.plain) {
            return Err(format!(
                "{when}: the history holds another change under what plays"
            ));
        }
        Ok(())
    }
}

/// Every instance's progress is a progress.
fn progress_is_whole(doc: &Document, when: &str) -> Result<(), String> {
    for instance in doc.presentations() {
        if !(0.0..=1.0).contains(&instance.t) {
            return Err(format!(
                "{when}: `{}` is at progress {}",
                instance.presentation, instance.t
            ));
        }
    }
    Ok(())
}

/// A performance for another surface's properties to run under: on
/// every one of `anchors`, one held presentation per target, writing
/// every mutation `PLAYABLE` lists for it — positions, sizes,
/// opacities, colors, text, visibility. Returns `map` with it playing.
pub fn performed(map: &Document, anchors: &[NodeId]) -> Result<Document, String> {
    let mut performed = map.clone();
    for (target, mutations) in PLAYABLE {
        let file = format!(
            r#"{{"presentations":[{{"id":"on-{target}","name":"P","play":"hold","target":"{target}","mutations":[{}]}}]}}"#,
            mutations.join(",")
        );
        let def = parse_presentations(&file)
            .map_err(|error| format!("the performance reads: {error}"))?
            .remove(0);
        performed
            .start_presentation(&def, anchors, 0.0)
            .map_err(|error| format!("the performance starts: {error}"))?;
    }
    // It shows, and none of it is in a save: a performance that changed
    // nothing would hold a property run under it to nothing.
    let shows = performed
        .presentations()
        .filter(|instance| !instance.elements.is_empty())
        .count();
    if shows < anchors.len() {
        return Err(format!(
            "the performance plays on {shows} instances, for {} nodes",
            anchors.len()
        ));
    }
    Maps::beside(map.clone(), performed.clone())?.same_map("with the performance on")?;
    Ok(performed)
}

/// The differential, for one edit (ruling D2.1 §2 on #251): run on
/// `plain` and on `performed` — the same map with presentations playing
/// — it leaves the same history, image for image, and the same saved
/// bytes, and so do its undo and its redo. The edit is built from the
/// map itself when `from_the_map`, as every funnel of the app builds a
/// change; from the map as it is drawn otherwise, which is the control.
pub fn check_edit_under(
    plain: Document,
    performed: Document,
    edit: &EditDef,
    anchors: &[NodeId],
    from_the_map: bool,
) -> Result<(), String> {
    let mut maps = Maps::beside(plain, performed)?;
    edit_under(&mut maps, edit, anchors, from_the_map).map(|_| ())
}

/// Run `edit` on both maps — built from the map itself when
/// `from_the_map`, with the layer set aside — and hold the two to each
/// other after it, after its undo and after its redo. Returns whether
/// it changed the map.
fn edit_under(
    maps: &mut Maps,
    edit: &EditDef,
    anchors: &[NodeId],
    from_the_map: bool,
) -> Result<bool, String> {
    let handlers = registry();
    let request = |map: &Document| {
        edit.request(map, &Boxes::of(map), &handlers, anchors)
            .map_err(|error| error.to_string())
    };
    let here = request(&maps.plain);
    let there = if from_the_map {
        maps.performed.suspend_presentations();
        let there = request(&maps.performed);
        maps.performed.resume_presentations();
        there
    } else {
        request(&maps.performed)
    };
    let (here, there) = match (here, there) {
        (Ok(here), Ok(there)) => (here, there),
        (Err(a), Err(b)) if a == b => return Ok(false),
        (a, b) => {
            return Err(format!(
                "edit `{}` resolved apart under what plays: {:?} and {:?}",
                edit.id,
                a.err(),
                b.err()
            ))
        }
    };
    let revision = maps.plain.revision();
    let (ran, also) = (
        maps.plain.apply(here, ChangeSource::Edit),
        maps.performed.apply(there, ChangeSource::Edit),
    );
    if ran != also {
        return Err(format!(
            "edit `{}` applied apart under what plays: {ran:?} and {also:?}",
            edit.id
        ));
    }
    maps.same_change(&format!("after edit `{}`", edit.id))?;
    if maps.plain.revision() == revision {
        return Ok(false);
    }
    if maps.plain.undo() != maps.performed.undo() {
        return Err(format!("edit `{}`: its undo went apart", edit.id));
    }
    maps.same_change(&format!("after undoing edit `{}`", edit.id))?;
    if maps.plain.redo() != maps.performed.redo() {
        return Err(format!("edit `{}`: its redo went apart", edit.id));
    }
    maps.same_change(&format!("after redoing edit `{}`", edit.id))?;
    Ok(true)
}

/// The properties, on the presentations file `text`; `generated` when
/// it is one of [`generate`]'s, which are always JSON of the file's
/// shape, so a refusal must be inside a presentation.
///
/// The first [`MAX_PLAYED`] definitions are started on the map's
/// top-level nodes, one on top of the other; then the layer is brought
/// to `frames` and to the edges of its first definitions' timings, with
/// one of `edits` run under it after each frame; then a long while
/// passes; then every one is stopped, and [`LIFECYCLE_MS`] later
/// nothing plays.
fn check_text(
    text: &str,
    generated: bool,
    edits: &[EditDef],
    frames: &[f64],
    from_the_map: bool,
) -> Result<Played, String> {
    let defs = match parse_presentations(text) {
        Ok(defs) => defs,
        Err(error) => {
            let inside = error.path.starts_with("presentations[");
            if generated && !inside {
                return Err(format!(
                    "a generated presentation was refused outside its path: {error}\n{text}"
                ));
            }
            // A refusal with no path is about the whole text: one that is
            // not JSON, or not an object, or one that lacks
            // `presentations`.
            let about_the_whole = matches!(
                error.kind,
                LoadErrorKind::Syntax { .. }
                    | LoadErrorKind::Invalid { .. }
                    | LoadErrorKind::MissingKey { .. }
                    | LoadErrorKind::ByteOrderMark
            );
            if !generated && error.path.is_empty() && !about_the_whole {
                return Err(format!("a refusal names no path into the file: {error}"));
            }
            return Ok(Played::default());
        }
    };
    let map = load(MAP).map_err(|error| format!("the map loads: {error}"))?;
    let anchors: Vec<NodeId> = map
        .children(map.root())
        .filter(|&id| map.get(id).is_some_and(|e| e.kind() == DocKind::Node))
        .collect();
    let mut maps = Maps::of(map)?;
    let mut played = Played::default();
    let mut readings: Vec<f64> = frames.to_vec();
    for def in defs.iter().take(MAX_PLAYED) {
        if let PresentationBody::Glyph(ops) = &def.body {
            check_glyph_body(ops)
                .map_err(|why| format!("presentation `{}`: its run mutations: {why}", def.id))?;
        }
        let playing = maps.performed.presentations().count();
        if maps.performed.start_presentation(def, &anchors, 0.0).is_err() {
            // All or nothing: a refused start starts no instance.
            if maps.performed.presentations().count() != playing {
                return Err(format!("presentation `{}` was refused, and plays", def.id));
            }
            continue;
        }
        if maps.performed.presentations().count() == playing {
            continue;
        }
        played.started += 1;
        // The edges of the first two timings that play: where a
        // direction begins, where it is half way, where it ends.
        if played.started <= 2 {
            readings.extend([def.animate, def.out()].iter().flat_map(|animate| {
                let (delay, length) = (f64::from(animate.delay_ms), f64::from(animate.ms));
                [delay, delay + length / 2.0, delay + length]
            }));
        }
    }
    maps.same_map("after the starts")?;
    readings.sort_by(f64::total_cmp);
    for (at, reading) in readings.iter().enumerate() {
        maps.performed.tick_presentations(*reading);
        progress_is_whole(&maps.performed, "at a frame")?;
        maps.same_save("at a frame")?;
        // An edit under what plays, between two frames.
        if let Some(edit) = edits.get(at) {
            let changed = edit_under(&mut maps, edit, &anchors, from_the_map)?;
            played.edited += usize::from(changed && maps.performed.presentations().count() > 0);
        }
    }
    // One frame, a million million milliseconds on: a loop has run
    // billions of cycles, and none of them is walked.
    let mut now = 1e12_f64;
    maps.performed.tick_presentations(now);
    progress_is_whole(&maps.performed, "a long while on")?;
    for instance in maps.performed.presentations() {
        if instance.moving && !instance.unending {
            return Err(format!(
                "`{}` is still on its way a long while after it started, and is no loop",
                instance.presentation
            ));
        }
    }
    maps.same_map("a long while on")?;
    // Stopped, each is over within its lifecycle.
    for def in defs.iter().take(MAX_PLAYED) {
        maps.performed.stop_presentation(&def.id, &anchors, now);
    }
    maps.same_save("after the stops")?;
    now += LIFECYCLE_MS;
    maps.performed.tick_presentations(now);
    if let Some(instance) = maps.performed.presentations().next() {
        return Err(format!(
            "`{}` still plays {LIFECYCLE_MS} ms after it was stopped",
            instance.presentation
        ));
    }
    maps.same_map("once all is over")?;
    Ok(played)
}

/// How a presentation lives, as the file spells it, and a spelling it
/// does not have.
const PLAYS: [&str; 5] = ["while", "once", "loop", "hold", "toggle"];

/// Run mutations as a glyph-layer body spells them, each one a body
/// takes.
pub(crate) const GLYPH_OPS: [&str; 9] = [
    r#"{"run_opacity":0.5}"#,
    r#"{"run_opacity":0.0}"#,
    r#"{"run_origin":[12.0,-4.0]}"#,
    r#"{"run_overrides":{"offset":{"range":[0,3],"value":[0.0,-2.0]}}}"#,
    r#"{"run_overrides":{"offset":{"range":[0,18446744073709551615],"value":[1.0,1.0]}}}"#,
    r##"{"run_overrides":{"color":{"range":[0,2],"value":"#ff000080"}}}"##,
    r#"{"run_overrides":{"color":{"range":[1,4],"value":null}}}"#,
    r#"{"run_overrides":{"scale":{"range":[0,6],"value":1.15}}}"#,
    r#"{"run_overrides":"clear"}"#,
];

/// Run mutations no body takes: past a bound, a range that runs
/// backward, a color that is a name, a mutation with no spelling.
pub(crate) const BAD_GLYPH_OPS: [&str; 7] = [
    r#"{"run_opacity":1.5}"#,
    r#"{"run_opacity":1e39}"#,
    r#"{"run_origin":[1e39,0.0]}"#,
    r#"{"run_overrides":{"offset":{"range":[3,1],"value":[1e30,0.0]}}}"#,
    r#"{"run_overrides":{"color":{"range":[0,2],"value":"var(accent)"}}}"#,
    r#"{"run_overrides":{"scale":{"range":[0,6],"value":-1.0}}}"#,
    r#"{"RunSet":{}}"#,
];

/// What a presentation plays on, as the file spells it, each with
/// mutations the fixture's elements of that kind take.
pub(crate) const PLAYABLE: [(&str, &[&str]); 8] = [
    (
        "node",
        &[
            r#"{"node":{"nudge":[40.0,-25.0]}}"#,
            r#"{"node":{"set_opacity":0.25}}"#,
            r#"{"node":{"set_hidden":true}}"#,
            r#"{"node":"toggle_folded"}"#,
            r#"{"node":{"text":{"set_size":31.0}}}"#,
            r#"{"node":{"set_palette":"mono"}}"#,
        ],
    ),
    (
        "sections",
        &[
            r#"{"section":{"nudge":[5.0,9.0]}}"#,
            r#"{"section":{"set_align":"center"}}"#,
            r##"{"section":{"text":{"set_color":"#654321"}}}"##,
        ],
    ),
    (
        "spans",
        &[
            r#"{"span":{"set_text":"presented"}}"#,
            r#"{"span":{"text":{"grow_size":9.0}}}"#,
            r#"{"span":{"text":{"set_color":"var(accent)"}}}"#,
        ],
    ),
    (
        "frame",
        &[
            r#"{"frame":{"set_visible":false}}"#,
            r#"{"frame":{"set_gap":7.0}}"#,
        ],
    ),
    ("rings", &[r##"{"ring":{"text":{"set_color":"#00ff00"}}}"##]),
    (
        "connector",
        &[
            r##"{"path":{"set_color":"#ff00ff"}}"##,
            r#"{"path":{"set_size":19.0}}"#,
        ],
    ),
    (
        "links",
        &[
            r##"{"path":{"set_color":"#ffff00"}}"##,
            r#"{"path":{"set_content":"+"}}"#,
        ],
    ),
    ("labels", &[r#"{"label":{"set_t":0.25}}"#]),
];

/// The targets whose elements draw runs of their own: what a
/// glyph-layer presentation may name.
const DRAWN: [&str; 5] = ["sections", "rings", "connector", "links", "labels"];

/// A presentations file of up to three presentations, each built from
/// `choices`.
pub fn generate(choices: &mut Choices<'_>) -> String {
    let count = 1 + choices.below(3);
    let presentations: Vec<String> = (0..count).map(|i| presentation(choices, i)).collect();
    format!(r#"{{"presentations":[{}]}}"#, presentations.join(","))
}

/// How a direction goes, mostly short, sometimes at the bound and one
/// past it, sometimes taking no time.
fn timing(choices: &mut Choices<'_>) -> String {
    let ms = match choices.below(16) {
        0 => MAX_ANIMATE_MS,
        1 => MAX_ANIMATE_MS + 1,
        2 => 0,
        _ => 1 + choices.below(1000),
    };
    format!(
        r#"{{"ms":{ms},"delay_ms":{},"easing":"{}","snap":"{}"}}"#,
        choices.below(100),
        choices.pick(&["linear", "ease_in", "ease_out", "ease_in_out"]),
        choices.pick(&["start", "end"])
    )
}

/// One presentation: a play, a layer or none, a target, a filter or
/// none, a body and its two timings or neither. Most are ones the
/// fixture plays — a target its layer takes, mutations its elements
/// take — and one choice in eight at each turn is not: a play no
/// presentation has, a target the glyph layer refuses, the other
/// layer's body, a mutation of another kind or past its bounds.
fn presentation(choices: &mut Choices<'_>, i: u32) -> String {
    let glyph = choices.one_in(3);
    let mut out = format!(
        r#"{{"id":"p{i}","name":"P","play":"{}""#,
        if choices.one_in(16) {
            PLAYS[4]
        } else {
            *choices.pick(&PLAYS[..4])
        },
    );
    if glyph {
        out.push_str(r#","layer":"glyph""#);
    } else if choices.one_in(2) {
        out.push_str(r#","layer":"document""#);
    }
    let (target, takes) = if glyph && !choices.one_in(8) {
        (*choices.pick(&DRAWN), &[][..])
    } else {
        *choices.pick(&PLAYABLE)
    };
    debug_assert!(TARGETS.contains(&target));
    out.push_str(&format!(r#","target":"{target}""#));
    if choices.one_in(8) {
        let fields: Vec<&str> = (0..choices.below(3)).map(|_| *choices.pick(&FIELDS)).collect();
        out.push_str(&format!(r#","filter":{{"fields":[{}]}}"#, fields.join(",")));
    }
    if glyph != choices.one_in(8) {
        let ops: Vec<&str> = (0..1 + choices.below(3))
            .map(|_| {
                if choices.one_in(8) {
                    *choices.pick(&BAD_GLYPH_OPS)
                } else {
                    *choices.pick(&GLYPH_OPS)
                }
            })
            .collect();
        out.push_str(&format!(r#","glyph":[{}]"#, ops.join(",")));
    } else {
        let mutations: Vec<&str> = (0..1 + choices.below(3))
            .map(|_| {
                if takes.is_empty() || choices.one_in(8) {
                    *choices.pick(&MUTATIONS)
                } else {
                    *choices.pick(takes)
                }
            })
            .collect();
        out.push_str(&format!(r#","mutations":[{}]"#, mutations.join(",")));
    }
    if choices.one_in(2) {
        out.push_str(&format!(r#","animate":{}"#, timing(choices)));
    }
    if choices.one_in(2) {
        out.push_str(&format!(r#","return":{}"#, timing(choices)));
    }
    out.push('}');
    out
}
