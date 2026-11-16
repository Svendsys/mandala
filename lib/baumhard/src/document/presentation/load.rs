// SPDX-License-Identifier: MPL-2.0

//! Reading presentations: a map's `presentations`, the bundled
//! `assets/presentations.json` and the user's `presentations.json` all
//! go through [`definition`], which holds an authored presentation to
//! the format's rules (`format/presentations.md`) and to the definition
//! bounds (owner, format Q4 on #251):
//!
//! - its body is its layer's: `mutations` on the document layer, `glyph`
//!   on the glyph layer, exactly one of the two;
//! - every mutation applies to the target's kind, and means something
//!   on some map: it is made concrete against no map, as an edit's is,
//!   so a face this build lacks is refused here and a name only the
//!   open map can answer waits for the start. No mutation is structural:
//!   the format has no spelling for one, so a file that writes one does
//!   not read, and the document refuses one again when an instance
//!   starts (`validate_for_presentation`, D17);
//! - a glyph-layer presentation targets a kind that draws a run of its
//!   own — `sections`, `rings`, `connector`, `links`, `labels` — and its
//!   numbers are in bounds;
//! - no more than [`MAX_DEFINITION_PARTS`] parts (mutations, run
//!   mutations and filter tests), and animated for no longer than
//!   [`MAX_ANIMATE_MS`](crate::document::bounds::MAX_ANIMATE_MS) each
//!   way, with no longer a delay;
//! - a `loop` takes time: one whose way in and way out are both instant
//!   has no state at any instant.
//!
//! A presentation has no nested part, so the depth bound
//! ([`MAX_DEFINITION_DEPTH`](crate::document::bounds::MAX_DEFINITION_DEPTH))
//! is one no presentation the format reads can reach; a text nested past
//! what the JSON reader follows is refused as too deep a definition
//! before the reader reads it
//! ([`check_nesting`]), so
//! the refusal a user meets is never the reader's.

use crate::document::bounds::{
    check_animate_ms, check_canvas_coord, check_non_negative, check_unit_interval, BoundError,
    MAX_DEFINITION_PARTS,
};
use crate::document::edit::load::check_nesting;
use crate::document::edit::resolve::{self, Fault, NameKind};
use crate::document::edit::{Animate, Easing, Filter, Mutations, Target};
use crate::document::format::edits::{AnimateDto, SnapDto};
use crate::document::format::error::{render_path, LoadError, LoadErrorKind, Seg};
use crate::document::format::index::parse_extent;
use crate::document::format::presentations::{
    GlyphOpDto, LayerDto, PresentationDto, PresentationsFileDto, RunOverrideDto,
};
use crate::document::keyed::KeyedSet;
use crate::document::place::{name_fault, NameFault};
use crate::document::presentation::{
    takes_time, GlyphOps, PresentationBody, PresentationDef, PresentationLayer, INSTANT, LOOP_TAKES_NO_TIME,
};
use crate::engine::interpolate::Snap;
use serde_json::value::RawValue;

/// Why a presentations file did not read.
pub type PresentationsError = LoadError;

/// The presentations of a file outside a map —
/// `{ "presentations": [ … ] }`, the bundled
/// `assets/presentations.json` and the user's `presentations.json` —
/// each held to the rules a map's are, ids unique.
///
/// Errors: the first refusal, with its JSON path and place in the text.
///
/// Cost: O(text), and each definition's own size.
pub fn parse_presentations(json: &str) -> Result<Vec<PresentationDef>, PresentationsError> {
    /// The file's `presentations`, captured whatever its depth. A file
    /// this does not read is the strict read's to refuse, below.
    #[derive(serde::Deserialize)]
    struct Extent<'t> {
        #[serde(borrow)]
        presentations: Option<&'t RawValue>,
    }
    let whole: &RawValue = serde_json::from_str(json).map_err(|error| {
        LoadError::new(
            String::new(),
            LoadErrorKind::Syntax {
                message: error.to_string(),
            },
        )
    })?;
    if let Ok(Extent {
        presentations: Some(list),
    }) = serde_json::from_str(whole.get())
    {
        check_nesting(json, list, "presentations")?;
    }
    let file: PresentationsFileDto = parse_extent(json, whole, &[])?;
    let mut seen: KeyedSet<String> = KeyedSet::default();
    let mut presentations = Vec::with_capacity(file.presentations.len());
    for (i, dto) in file.presentations.into_iter().enumerate() {
        let at = |segs: Vec<Seg>, kind| {
            let mut path = vec![Seg::Key("presentations"), Seg::Index(i)];
            path.extend(segs);
            LoadError::new(render_path(&path), kind)
        };
        if let Some(fault) = name_fault(&dto.id) {
            let kind = match fault {
                NameFault::Empty => LoadErrorKind::EmptyName,
                NameFault::NotPrintable(c) => LoadErrorKind::NameNotPrintable {
                    code_point: u32::from(c),
                },
            };
            return Err(at(vec![Seg::Key("id")], kind));
        }
        if !seen.insert(dto.id.clone()) {
            return Err(at(
                vec![Seg::Key("id")],
                LoadErrorKind::DuplicateDefinition { id: dto.id },
            ));
        }
        presentations.push(definition(dto).map_err(|(segs, kind)| at(segs, kind))?);
    }
    Ok(presentations)
}

/// The kinds a glyph-layer presentation may target: the ones that draw
/// a run of their own.
const GLYPH_TARGETS: [Target; 5] = [
    Target::Sections,
    Target::Rings,
    Target::Connector,
    Target::Links,
    Target::Labels,
];

/// `dto` held to the rules in the module docs, as a
/// [`PresentationDef`]; or the path inside the definition and what is
/// wrong there.
pub(crate) fn definition(dto: PresentationDto) -> Result<PresentationDef, (Vec<Seg>, LoadErrorKind)> {
    let target: Target = dto.target.into();
    let kind = target.kind();
    let layer: PresentationLayer = dto.layer.unwrap_or(LayerDto::Document).into();
    let invalid = |segs: Vec<Seg>, message: String| (segs, LoadErrorKind::InvalidPresentation { message });
    let mut parts = 0usize;
    let filter = match dto.filter {
        Some(filter) => {
            parts += filter.fields.len();
            resolve::predicate(&filter, None).map_err(|f| fault(vec![Seg::Key("filter")], f))?;
            Some(Filter(filter))
        }
        None => None,
    };
    let body = match (layer, dto.mutations, dto.glyph) {
        (PresentationLayer::Document, Some(mutations), None) => {
            parts += mutations.len();
            for (i, mutation) in mutations.iter().enumerate() {
                let at = || vec![Seg::Key("mutations"), Seg::Index(i)];
                if !mutation.applies_to(kind) {
                    return Err(invalid(
                        at(),
                        format!("this mutation does not apply to the presentation's target, `{kind:?}`"),
                    ));
                }
                resolve::mutation(mutation, None).map_err(|f| fault(at(), f))?;
            }
            PresentationBody::Mutations(Mutations(mutations))
        }
        (PresentationLayer::Glyph, None, Some(ops)) => {
            if !GLYPH_TARGETS.contains(&target) {
                let accepted: Vec<String> = GLYPH_TARGETS
                    .iter()
                    .map(|t| format!("`{}`", t.spelling()))
                    .collect();
                return Err(invalid(
                    vec![Seg::Key("target")],
                    format!(
                        "a glyph-layer presentation plays on the runs its target draws, and `{}` draws \
                         none of its own: its target is one of {}",
                        target.spelling(),
                        accepted.join(", ")
                    ),
                ));
            }
            parts += ops.len();
            for (i, op) in ops.iter().enumerate() {
                glyph_op(op).map_err(|(mut segs, error)| {
                    let mut at = vec![Seg::Key("glyph"), Seg::Index(i)];
                    at.append(&mut segs);
                    (at, LoadErrorKind::Bound { error })
                })?;
            }
            PresentationBody::Glyph(GlyphOps(ops))
        }
        (PresentationLayer::Document, mutations, glyph) => {
            return Err(invalid(
                Vec::new(),
                format!(
                    "a document-layer presentation has `mutations` and no `glyph`; this one has {}",
                    bodies(mutations.is_some(), glyph.is_some())
                ),
            ));
        }
        (PresentationLayer::Glyph, mutations, glyph) => {
            return Err(invalid(
                Vec::new(),
                format!(
                    "a glyph-layer presentation has `glyph` and no `mutations`; this one has {}",
                    bodies(mutations.is_some(), glyph.is_some())
                ),
            ));
        }
    };
    if parts > MAX_DEFINITION_PARTS {
        return Err((
            Vec::new(),
            LoadErrorKind::DefinitionTooLarge {
                max: MAX_DEFINITION_PARTS,
            },
        ));
    }
    let animate = timing("animate", dto.animate.as_ref())?.unwrap_or(INSTANT);
    let ret = timing("return", dto.ret.as_ref())?;
    let play = dto.play.into();
    if !takes_time(play, &animate, &ret.unwrap_or(animate)) {
        return Err(invalid(Vec::new(), LOOP_TAKES_NO_TIME.to_string()));
    }
    Ok(PresentationDef {
        id: dto.id,
        name: dto.name,
        description: dto.description,
        play,
        layer,
        target,
        filter,
        body,
        animate,
        ret,
    })
}

/// Which bodies a definition has, in words, for a refusal.
fn bodies(mutations: bool, glyph: bool) -> &'static str {
    match (mutations, glyph) {
        (true, true) => "both",
        (true, false) => "`mutations`",
        (false, true) => "`glyph`",
        (false, false) => "neither",
    }
}

/// One direction's timing under `key`, held to the bound on its
/// duration and its delay. An authored timing that leaves out `snap`
/// reads as `end` (design §4.6): the format's default, never the
/// engine's.
fn timing(key: &'static str, dto: Option<&AnimateDto>) -> Result<Option<Animate>, (Vec<Seg>, LoadErrorKind)> {
    let Some(dto) = dto else {
        return Ok(None);
    };
    for (field, ms) in [("ms", dto.ms), ("delay_ms", dto.delay_ms)] {
        check_animate_ms(ms).map_err(|error| {
            (
                vec![Seg::Key(key), Seg::Key(field)],
                LoadErrorKind::Bound { error },
            )
        })?;
    }
    Ok(Some(Animate {
        ms: dto.ms,
        delay_ms: dto.delay_ms,
        easing: dto.easing.map_or(Easing::Linear, Into::into),
        snap: dto.snap.unwrap_or(SnapDto::End).into(),
    }))
}

/// Hold one run mutation's numbers to their bounds: an opacity in
/// `0..=1`, an origin and an offset on the canvas, a scale not below
/// zero. The path of the number inside the op, and the bound it broke.
fn glyph_op(op: &GlyphOpDto) -> Result<(), (Vec<Seg>, BoundError)> {
    let on_canvas = |key: &'static str, [x, y]: [f32; 2], path: &[Seg]| {
        for (axis, value) in [x, y].into_iter().enumerate() {
            check_canvas_coord(value).map_err(|error| {
                let mut segs = path.to_vec();
                segs.extend([Seg::Key(key), Seg::Index(axis)]);
                (segs, error)
            })?;
        }
        Ok(())
    };
    match op {
        GlyphOpDto::Opacity(opacity) => check_unit_interval(*opacity)
            .map(drop)
            .map_err(|error| (vec![Seg::Key("run_opacity")], error)),
        GlyphOpDto::Origin(origin) => on_canvas("run_origin", *origin, &[]),
        GlyphOpDto::Overrides(RunOverrideDto::Offset { range: _, value }) => {
            on_canvas("value", *value, &[Seg::Key("run_overrides"), Seg::Key("offset")])
        }
        GlyphOpDto::Overrides(RunOverrideDto::Scale { range: _, value }) => {
            check_non_negative(*value).map(drop).map_err(|error| {
                (
                    vec![Seg::Key("run_overrides"), Seg::Key("scale"), Seg::Key("value")],
                    error,
                )
            })
        }
        GlyphOpDto::Overrides(RunOverrideDto::Color { range: _, value: _ } | RunOverrideDto::Clear) => Ok(()),
    }
}

/// `def` as a file writes it: what the saver writes back. A direction
/// that is the default — an instant way in, a way out as the way in —
/// is written by leaving its key out.
pub(crate) fn to_dto(def: &PresentationDef) -> PresentationDto {
    let (mutations, glyph) = match &def.body {
        PresentationBody::Mutations(list) => (Some(list.0.clone()), None),
        PresentationBody::Glyph(ops) => (None, Some(ops.0.clone())),
    };
    let timing = |a: Animate| AnimateDto {
        ms: a.ms,
        delay_ms: a.delay_ms,
        easing: (a.easing != Easing::Linear).then(|| a.easing.into()),
        snap: (a.snap != Snap::End).then(|| a.snap.into()),
    };
    PresentationDto {
        id: def.id.clone(),
        name: def.name.clone(),
        description: def.description.clone(),
        play: def.play.into(),
        layer: (def.layer != PresentationLayer::Document).then(|| def.layer.into()),
        target: def.target.into(),
        filter: def.filter.as_ref().map(|filter| filter.0.clone()),
        mutations,
        glyph,
        animate: (def.animate != INSTANT).then(|| timing(def.animate)),
        ret: def.ret.map(timing),
    }
}

/// A refusal of a resolution at load, under `segs`.
fn fault(segs: Vec<Seg>, fault: Fault) -> (Vec<Seg>, LoadErrorKind) {
    let kind = match fault {
        Fault::Unknown {
            kind: NameKind::Font,
            name,
            ..
        } => LoadErrorKind::UnknownFont { family: name },
        // Every other name answers at load, against no map.
        Fault::Unknown { field, name, .. } => LoadErrorKind::InvalidPresentation {
            message: format!("`{field}`: `{name}` is not a name"),
        },
        Fault::Bound { error, .. } => LoadErrorKind::Bound { error },
        Fault::Invalid { message, .. } => LoadErrorKind::InvalidPresentation {
            message: message.to_string(),
        },
    };
    (segs, kind)
}
