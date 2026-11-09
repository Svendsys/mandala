// SPDX-License-Identifier: MPL-2.0

//! Reading edits: a map's `edits`, the bundled `assets/edits.json` and
//! the user's `edits.json` all go through [`definition`], which holds an
//! authored edit to the format's rules (`format/edits.md`) and to the
//! definition bounds (owner, format Q4 on #250):
//!
//! - exactly one of `mutations`, `mutator` and `handler`;
//! - every payload applies to the element it is aimed at — a mutation to
//!   the target's kind, a macro's to its key's — and a mutator's root
//!   has the target's kind, which is the only kind it can be anchored
//!   at;
//! - a handler is one this build has, and is aimed at `node`;
//! - a mutator that reaches below its root is not aimed at nodes through
//!   `descendants` or `subtree`, whose nodes nest: the walker starts
//!   only from the outermost of nested anchors;
//! - no deeper than [`MAX_DEFINITION_DEPTH`] levels and no more than
//!   [`MAX_DEFINITION_PARTS`] parts, and animated for no longer than
//!   [`MAX_ANIMATE_MS`](crate::document::bounds::MAX_ANIMATE_MS), with
//!   no longer a delay;
//! - every value means something on some map: it is made concrete
//!   against no map, where every map-specific name answers, so a
//!   face this build lacks or a radius past 100 is refused here, and a
//!   name only the open map can answer waits for the run.
//!
//! The JSON reader follows 128 levels; a definition at the depth bound
//! is far inside that, and [`check_nesting`] refuses anything deeper
//! inside `edits` before the reader sees it, so the refusal a user meets
//! is the definition bound, never the reader's.

use crate::document::bounds::{check_animate_ms, MAX_DEFINITION_DEPTH, MAX_DEFINITION_PARTS};
use crate::document::edit::resolve::{self, Fault, NameKind};
use crate::document::edit::{Animate, EditBody, EditDef, Filter, Mutations, MutatorAst, Target};
use crate::document::format::context::position_at;
use crate::document::format::edits::{EditDto, EditsFileDto, InstructionDto, MutatorDto, ReachDto, SnapDto};
use crate::document::format::error::{render_path, LoadError, LoadErrorKind, Seg};
use crate::document::format::index::parse_extent;
use crate::document::keyed::KeyedSet;
use crate::document::place::{name_fault, NameFault};
use crate::engine::builder::{build, NoRuntime};
use serde_json::value::RawValue;

/// The deepest JSON nesting inside a map's `edits`, or an edits file,
/// that a definition within the bounds can reach: three levels per
/// mutator level (`{"void": {"children": [ … ]}}`), the list and the
/// definition around them, and the deepest leaf — a `repeat_while`
/// test, `{"instruction": {"repeat_while": {"fields": [[{"fit": {"fit":
/// {…}}}, …]]}}}` — with room to spare. Below the reader's 128, so a text
/// deeper than this is refused as too deep a definition before the
/// reader would stop at its own limit.
pub(crate) const MAX_EDITS_JSON_DEPTH: usize = 3 * MAX_DEFINITION_DEPTH + 16;

/// Why an edits file did not read.
pub type EditsError = LoadError;

/// The edits of a file outside a map — `{ "edits": [ … ] }`, the bundled
/// `assets/edits.json` and the user's `edits.json` — each held to the
/// rules a map's are, ids unique.
///
/// Errors: the first refusal, with its JSON path and place in the text.
///
/// Cost: O(text), and each definition's own size.
pub fn parse_edits(json: &str) -> Result<Vec<EditDef>, EditsError> {
    check_nesting(json)?;
    let whole: &RawValue = serde_json::from_str(json).map_err(|error| {
        LoadError::new(
            String::new(),
            LoadErrorKind::Syntax {
                message: error.to_string(),
            },
        )
    })?;
    let file: EditsFileDto = parse_extent(json, whole, &[])?;
    let mut seen: KeyedSet<String> = KeyedSet::default();
    let mut edits = Vec::with_capacity(file.edits.len());
    for (i, dto) in file.edits.into_iter().enumerate() {
        let at = |segs: Vec<Seg>, kind| {
            let mut path = vec![Seg::Key("edits"), Seg::Index(i)];
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
        edits.push(definition(dto).map_err(|(segs, kind)| at(segs, kind))?);
    }
    Ok(edits)
}

/// Refuse a text — a map, or an edits file — whose top-level `edits`
/// nest deeper than [`MAX_EDITS_JSON_DEPTH`], at the definition that
/// does, before the JSON reader meets it. Nesting anywhere else, and a
/// text that is not JSON, is left to the reader. O(text).
pub(crate) fn check_nesting(json: &str) -> Result<(), LoadError> {
    let bytes = json.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    // The top-level key being read, and the entry of its array.
    let mut key_start = None;
    let mut last_string: (usize, usize) = (0, 0);
    let mut key: (usize, usize) = (0, 0);
    let mut entry = 0usize;
    for (offset, &byte) in bytes.iter().enumerate() {
        if in_string {
            match (escaped, byte) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => {
                    in_string = false;
                    if let Some(start) = key_start.take() {
                        last_string = (start, offset);
                    }
                }
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                if depth == 1 {
                    key_start = Some(offset + 1);
                }
            }
            b':' if depth == 1 => {
                key = last_string;
                entry = 0;
            }
            b',' if depth == 2 => entry += 1,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_EDITS_JSON_DEPTH + 1 {
                    let name = json.get(key.0..key.1).unwrap_or("");
                    if name != "edits" {
                        // Not a definition: the reader's to refuse.
                        return Ok(());
                    }
                    let mut error = LoadError::new(
                        render_path(&[Seg::Key("edits"), Seg::Index(entry)]),
                        LoadErrorKind::DefinitionTooDeep {
                            max: MAX_DEFINITION_DEPTH,
                        },
                    );
                    error.position = Some(position_at(json, offset));
                    return Err(error);
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// `dto` held to the rules in the module docs, as an [`EditDef`]; or
/// the path inside the definition and what is wrong there.
pub(crate) fn definition(dto: EditDto) -> Result<EditDef, (Vec<Seg>, LoadErrorKind)> {
    let target: Target = dto.target.into();
    let kind = target.kind();
    let invalid = |segs: Vec<Seg>, message: String| (segs, LoadErrorKind::InvalidEdit { message });
    let bodies = usize::from(dto.mutations.is_some())
        + usize::from(dto.mutator.is_some())
        + usize::from(dto.handler.is_some());
    if bodies != 1 {
        return Err(invalid(
            Vec::new(),
            format!("an edit has exactly one of `mutations`, `mutator` and `handler`; this one has {bodies}"),
        ));
    }
    let mut parts = 0usize;
    let filter = match dto.filter {
        Some(filter) => {
            parts += filter.fields.len();
            resolve::predicate(&filter, None).map_err(|f| fault(vec![Seg::Key("filter")], f))?;
            Some(Filter(filter))
        }
        None => None,
    };
    let body = if let Some(mutations) = dto.mutations {
        parts += mutations.len();
        for (i, mutation) in mutations.iter().enumerate() {
            let at = || vec![Seg::Key("mutations"), Seg::Index(i)];
            if !mutation.applies_to(kind) {
                return Err(invalid(
                    at(),
                    format!("this mutation does not apply to the edit's target, `{kind:?}`"),
                ));
            }
            resolve::mutation(mutation, None).map_err(|f| fault(at(), f))?;
        }
        EditBody::Mutations(Mutations(mutations))
    } else if let Some(mutator) = dto.mutator {
        let root = mutator_kind(&mutator);
        if root != kind {
            return Err(invalid(
                vec![Seg::Key("mutator")],
                format!("the mutator's root is a `{root:?}`, but the edit's target is `{kind:?}`, where it is anchored"),
            ));
        }
        measure(&mutator, 1, &mut parts, &mut vec![Seg::Key("mutator")])?;
        let node = resolve::mutator(&mutator, None).map_err(|f| fault(vec![Seg::Key("mutator")], f))?;
        let nested = target == Target::Node && matches!(dto.reach, ReachDto::Descendants | ReachDto::Subtree);
        if nested && build(&node, &NoRuntime).is_ok_and(|tree| tree.reaches_below_root()) {
            return Err(invalid(
                vec![Seg::Key("mutator")],
                "a mutator that reaches below its root walks only from the outermost of the nodes \
                 `descendants` and `subtree` reach, so the others would be changed only where it \
                 reaches them: reach `self` and let the mutator walk down, or use `mutations`"
                    .to_string(),
            ));
        }
        EditBody::Mutator(MutatorAst(mutator))
    } else {
        let name = dto.handler.unwrap_or_default();
        if !crate::edit_handlers::exists(&name) {
            return Err(invalid(
                vec![Seg::Key("handler")],
                format!("this build has no handler `{name}`"),
            ));
        }
        if target != Target::Node {
            return Err(invalid(
                vec![Seg::Key("target")],
                "a handler lays out nodes: its target is `node`".to_string(),
            ));
        }
        EditBody::Handler(name)
    };
    if let Some(animate) = &dto.animate {
        for (key, ms) in [("ms", animate.ms), ("delay_ms", animate.delay_ms)] {
            check_animate_ms(ms).map_err(|error| {
                (
                    vec![Seg::Key("animate"), Seg::Key(key)],
                    LoadErrorKind::Bound { error },
                )
            })?;
        }
    }
    if parts > MAX_DEFINITION_PARTS {
        return Err((
            Vec::new(),
            LoadErrorKind::DefinitionTooLarge {
                max: MAX_DEFINITION_PARTS,
            },
        ));
    }
    Ok(EditDef {
        id: dto.id,
        name: dto.name,
        description: dto.description,
        reach: dto.reach.into(),
        target,
        filter,
        body,
        // An authored `animate` that leaves out `snap` reads as `end`
        // (design §4.6): the format's default, never the engine's.
        animate: dto.animate.map(|a| Animate {
            ms: a.ms,
            delay_ms: a.delay_ms,
            easing: a.easing.map_or(super::Easing::Linear, Into::into),
            snap: a.snap.unwrap_or(SnapDto::End).into(),
        }),
    })
}

/// `def` as a file writes it: what the saver writes back.
pub(crate) fn to_dto(def: &EditDef) -> EditDto {
    let (mutations, mutator, handler) = match &def.body {
        EditBody::Mutations(list) => (Some(list.0.clone()), None, None),
        EditBody::Mutator(ast) => (None, Some(ast.0.clone()), None),
        EditBody::Handler(name) => (None, None, Some(name.clone())),
    };
    EditDto {
        id: def.id.clone(),
        name: def.name.clone(),
        description: def.description.clone(),
        reach: def.reach.into(),
        target: def.target.into(),
        filter: def.filter.as_ref().map(|filter| filter.0.clone()),
        mutations,
        mutator,
        handler,
        animate: def.animate.map(|a| crate::document::format::edits::AnimateDto {
            ms: a.ms,
            delay_ms: a.delay_ms,
            easing: (a.easing != super::Easing::Linear).then(|| a.easing.into()),
            snap: (a.snap != crate::engine::interpolate::Snap::End).then(|| a.snap.into()),
        }),
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
        Fault::Unknown { field, name, .. } => LoadErrorKind::InvalidEdit {
            message: format!("`{field}`: `{name}` is not a name"),
        },
        Fault::Bound { error, .. } => LoadErrorKind::Bound { error },
        Fault::Invalid { message, .. } => LoadErrorKind::InvalidEdit {
            message: message.to_string(),
        },
    };
    (segs, kind)
}

/// The kind of a mutator node's key.
fn mutator_kind(node: &MutatorDto) -> crate::document::element::DocKind {
    match node {
        MutatorDto::Void { kind, .. }
        | MutatorDto::Macro { kind, .. }
        | MutatorDto::Instruction { kind, .. } => (*kind).into(),
    }
}

/// Hold `node`, at `depth` levels, to the depth bound, add its parts to
/// `parts`, and check each macro's mutations against its key's kind.
/// Recursive in the definition's depth, which it refuses past
/// [`MAX_DEFINITION_DEPTH`] before it recurses further.
fn measure(
    node: &MutatorDto,
    depth: usize,
    parts: &mut usize,
    path: &mut Vec<Seg>,
) -> Result<(), (Vec<Seg>, LoadErrorKind)> {
    if depth > MAX_DEFINITION_DEPTH {
        return Err((
            path.clone(),
            LoadErrorKind::DefinitionTooDeep {
                max: MAX_DEFINITION_DEPTH,
            },
        ));
    }
    *parts += 1;
    let (variant, children) = match node {
        MutatorDto::Void { children, .. } => ("void", children),
        MutatorDto::Macro {
            kind,
            mutations,
            children,
            ..
        } => {
            *parts += mutations.len();
            let kind: crate::document::element::DocKind = (*kind).into();
            for (i, mutation) in mutations.iter().enumerate() {
                if !mutation.applies_to(kind) {
                    let mut at = path.clone();
                    at.extend([Seg::Key("macro"), Seg::Key("mutations"), Seg::Index(i)]);
                    return Err((
                        at,
                        LoadErrorKind::InvalidEdit {
                            message: format!("this mutation does not apply to its macro's kind, `{kind:?}`"),
                        },
                    ));
                }
            }
            ("macro", children)
        }
        MutatorDto::Instruction {
            instruction,
            children,
            ..
        } => {
            if let InstructionDto::RepeatWhile(filter) = instruction {
                *parts += filter.fields.len();
            }
            ("instruction", children)
        }
    };
    if *parts > MAX_DEFINITION_PARTS {
        return Err((
            path.clone(),
            LoadErrorKind::DefinitionTooLarge {
                max: MAX_DEFINITION_PARTS,
            },
        ));
    }
    for (i, child) in children.iter().enumerate() {
        path.extend([Seg::Key(variant), Seg::Key("children"), Seg::Index(i)]);
        measure(child, depth + 1, parts, path)?;
        path.truncate(path.len() - 3);
    }
    Ok(())
}
