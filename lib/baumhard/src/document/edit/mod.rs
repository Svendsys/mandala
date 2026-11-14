// SPDX-License-Identifier: MPL-2.0

//! **Edits**: named, saved, undoable changes to a map (design §10.1, D6;
//! V1-D1, #250).
//!
//! An [`EditDef`](crate::document::edit::EditDef) names a set of
//! elements relative to the anchors it is run at — a
//! [`Reach`](crate::document::edit::Reach) of nodes, a
//! [`Target`](crate::document::edit::Target) within them and an optional
//! filter — and a body that changes them:
//!
//! - **mutations**, applied once to each element: one `Macro` at the
//!   root of the mutator, every element an anchor of one walk;
//! - **a mutator**, the V1-A1 AST, anchored at each element, reaching
//!   below it as its author wrote it;
//! - **a handler**, a Rust layout that reads the document and the drawn
//!   geometry and returns its walks — one mutator and its anchors per
//!   write it needs ([`edit_handlers`](crate::edit_handlers)). It
//!   is given `&Document`, so it cannot write: the type says so.
//!
//! [`EditDef::request`](crate::document::edit::EditDef::request) makes
//! any of the three one [`EditRequest`](crate::document::writer::EditRequest)
//! — a handler's walks are its steps —
//! which the caller applies (one history entry) or stages in an animated
//! transaction. Nothing here writes a document.
//!
//! An edit is read from a file with its names — theme variables,
//! palettes, fonts, verbs — and those names are resolved against the
//! open map when it runs (`document::format::edits`);
//! one that names something the map lacks refuses that run, naming the
//! edit, the field and the name, and nothing is applied.

pub(crate) mod load;
mod mentions;
mod reach;
mod resolve;

pub use load::{parse_edits, EditsError};
pub use mentions::Mentions;

use crate::arena::NodeId;
use crate::document::element::DocKind;
use crate::document::format::edits::{
    EasingDto, FilterDto, MutationDto, MutatorDto, ReachDto, SnapDto, TargetDto,
};
use crate::document::ids::Document;
use crate::document::mutation::ops::DocMutatorTree;
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::writer::EditRequest;
use crate::edit_handlers::Handlers;
use crate::engine::anchor::Anchors;
use crate::engine::builder::{build, NoRuntime};
use crate::engine::interpolate::Snap;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::tree::MutatorTree;
use crate::projection::Geometry;
use std::fmt;

/// Which nodes an edit reaches, relative to each anchor node.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// The anchor nodes themselves.
    SelfNode,
    /// Their child nodes.
    Children,
    /// Every node below them.
    Descendants,
    /// Them and every node below them.
    Subtree,
    /// Their parent nodes (a top-level node has none).
    Parent,
    /// The other nodes that share their parent.
    Siblings,
}

/// Which elements within the reached nodes an edit changes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The nodes.
    Node,
    /// Their sections.
    Sections,
    /// Every span of their sections.
    Spans,
    /// Their frames.
    Frame,
    /// Every ring of their frames.
    Rings,
    /// Their connectors.
    Connector,
    /// Their outgoing links.
    Links,
    /// The labels of their connectors and links.
    Labels,
}

impl Target {
    /// The kind of every element this target names. O(1).
    pub fn kind(self) -> DocKind {
        match self {
            Target::Node => DocKind::Node,
            Target::Sections => DocKind::Section,
            Target::Spans => DocKind::Span,
            Target::Frame => DocKind::Frame,
            Target::Rings => DocKind::Ring,
            Target::Connector => DocKind::Connector,
            Target::Links => DocKind::Link,
            Target::Labels => DocKind::Label,
        }
    }
}

/// How an animated edit's progress follows time.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Easing {
    /// Uniform.
    Linear,
    /// Slow at the start: `t²`.
    EaseIn,
    /// Slow at the end: `1 - (1 - t)²`.
    EaseOut,
    /// Slow at both ends: smoothstep, `t²(3 - 2t)`.
    EaseInOut,
}

impl Easing {
    /// The progress at time fraction `t`, clamped to `0..=1`. O(1).
    pub fn ease(self, t: f32) -> f32 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => t * t * (3.0 - 2.0 * t),
        }
    }
}

/// How an edit animates into place: it runs as a transaction, written
/// part-way at each frame, and commits as one history entry.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Animate {
    /// How long the change takes, in milliseconds.
    pub ms: u32,
    /// How long it holds at its start first.
    pub delay_ms: u32,
    /// How progress follows time.
    pub easing: Easing,
    /// When the fields that do not blend switch.
    pub snap: Snap,
}

impl Animate {
    /// The eased progress `elapsed_ms` after the edit started: `0` through
    /// the delay, `1` from its end on. O(1).
    pub fn progress(&self, elapsed_ms: f64) -> f32 {
        let running = elapsed_ms - f64::from(self.delay_ms);
        if running <= 0.0 {
            return 0.0;
        }
        if self.ms == 0 {
            return 1.0;
        }
        self.easing.ease((running / f64::from(self.ms)) as f32)
    }

    /// Whether the edit has run its course `elapsed_ms` after it started.
    /// O(1).
    pub fn is_done(&self, elapsed_ms: f64) -> bool {
        elapsed_ms >= f64::from(self.delay_ms) + f64::from(self.ms)
    }
}

/// A filter, as an edit writes it: a predicate whose names are resolved
/// when the edit runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter(pub(crate) FilterDto);

/// A mutation list, as an edit writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mutations(pub(crate) Vec<MutationDto>);

/// A mutator AST, as an edit writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct MutatorAst(pub(crate) MutatorDto);

/// What an edit does to the elements it reaches.
#[derive(Clone, Debug, PartialEq)]
pub enum EditBody {
    /// Each mutation, once on each element.
    Mutations(Mutations),
    /// A mutator anchored at each element.
    Mutator(MutatorAst),
    /// A handler, by name ([`edit_handlers`](crate::edit_handlers)).
    Handler(String),
}

/// An edit (design §10.1). See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct EditDef {
    /// The id keybinds, the console, macros and verbs name it by.
    pub id: String,
    /// The name shown to the user, and the history entry's label.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Which nodes, relative to each anchor node.
    pub reach: Reach,
    /// Which elements within them.
    pub target: Target,
    /// Which of those it keeps.
    pub filter: Option<Filter>,
    /// What it does to them.
    pub body: EditBody,
    /// How it animates; instant when `None`.
    pub animate: Option<Animate>,
}

/// Why an edit could not run. Nothing was applied.
#[derive(Clone, Debug, PartialEq)]
pub enum EditError {
    /// The edit names something the open map does not define.
    UnknownName {
        /// The edit's id.
        edit: String,
        /// The field that names it.
        field: &'static str,
        /// What it names, and as what.
        name: String,
    },
    /// A value the edit writes is not one its field takes.
    Value {
        /// The edit's id.
        edit: String,
        /// The field.
        field: &'static str,
        /// Why.
        message: String,
    },
    /// The edit names a handler this build does not have.
    UnknownHandler {
        /// The edit's id.
        edit: String,
        /// The handler's name.
        handler: String,
    },
    /// A handler declined the document it was given: its message, with
    /// the edit that ran it.
    Handler {
        /// The edit's id.
        edit: String,
        /// Why.
        message: String,
    },
    /// The mutator did not build. A load accepts only mutators that
    /// build, so this names a defect.
    Build {
        /// The edit's id.
        edit: String,
        /// Why.
        message: String,
    },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::UnknownName { edit, field, name } => {
                write!(
                    f,
                    "edit `{edit}`: `{field}` names {name}, which this map does not define"
                )
            }
            EditError::Value { edit, field, message } => write!(f, "edit `{edit}`: `{field}`: {message}"),
            EditError::UnknownHandler { edit, handler } => {
                write!(f, "edit `{edit}`: no handler is called `{handler}`")
            }
            EditError::Handler { edit, message } => write!(f, "edit `{edit}`: {message}"),
            EditError::Build { edit, message } => {
                write!(f, "edit `{edit}`: the mutator does not build: {message}")
            }
        }
    }
}

impl std::error::Error for EditError {}

impl EditDef {
    /// The request that runs this edit at `anchors` — selected elements,
    /// each standing for its owning node — on `doc`, as it stands: one
    /// walk, labeled with the edit's name. `geometry` is where things are
    /// drawn, which a handler reads; `handlers` is the registry.
    ///
    /// The names the edit writes are resolved against `doc` here; a name
    /// `doc` lacks refuses the run ([`EditError::UnknownName`]). A request
    /// whose walk reaches nothing changes nothing, and applying it makes
    /// no history entry.
    ///
    /// Cost: the reach's (the anchors, and the subtrees `descendants` and
    /// `subtree` walk), one resolution per authored mutation, and a
    /// handler's own.
    pub fn request(
        &self,
        doc: &Document,
        geometry: &dyn Geometry,
        handlers: &Handlers,
        anchors: &[NodeId],
    ) -> Result<EditRequest, EditError> {
        let steps = self
            .walks(doc, geometry, handlers, anchors)?
            .into_iter()
            .map(DocStep::fixed)
            .collect();
        Ok(EditRequest::new(steps, self.name.clone()))
    }

    /// The elements this edit would change at `anchors` on `doc`: the
    /// resolved reach, target and filter, in tree order. For a handler,
    /// the nodes it is given. What `edit inspect` reports.
    pub fn elements(&self, doc: &Document, anchors: &[NodeId]) -> Result<Vec<NodeId>, EditError> {
        let filter = match &self.filter {
            Some(filter) => {
                Some(resolve::predicate(&filter.0, Some(doc)).map_err(|f| f.into_error(&self.id))?)
            }
            None => None,
        };
        Ok(reach::elements(
            doc,
            anchors,
            self.reach,
            self.target,
            filter.as_ref(),
        ))
    }

    /// The body as a file writes it, in one line of JSON: what `edit
    /// inspect` shows. A handler's is `{"handler":"<name>"}`.
    pub fn body_json(&self) -> String {
        let written = match &self.body {
            EditBody::Mutations(list) => serde_json::to_string(&list.0),
            EditBody::Mutator(ast) => serde_json::to_string(&ast.0),
            EditBody::Handler(name) => {
                serde_json::to_string(name).map(|name| format!(r#"{{"handler":{name}}}"#))
            }
        };
        written.unwrap_or_else(|error| format!("<{error}>"))
    }

    /// The walks of [`Self::request`] at `anchors`: one for a
    /// declarative edit, anchored at [`Self::elements`]; a handler's,
    /// one per write it needs (ruling on #250), from running it. What
    /// `edit inspect` shows. Reads only.
    ///
    /// Errors: as [`Self::request`]'s.
    ///
    /// Cost: [`Self::elements`], then resolving the body or running the
    /// handler.
    pub fn walks(
        &self,
        doc: &Document,
        geometry: &dyn Geometry,
        handlers: &Handlers,
        anchors: &[NodeId],
    ) -> Result<Vec<DocWalk>, EditError> {
        let elements = self.elements(doc, anchors)?;
        let mutator: DocMutatorTree = match &self.body {
            EditBody::Mutations(list) => {
                let mutations = list
                    .0
                    .iter()
                    .map(|m| resolve::mutation(m, Some(doc)))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|f| f.into_error(&self.id))?;
                MutatorTree::new_with(Mutator::new_macro(
                    mutations,
                    BranchKey::new(self.target.kind(), 0),
                ))
            }
            EditBody::Mutator(ast) => {
                let node = resolve::mutator(&ast.0, Some(doc)).map_err(|f| f.into_error(&self.id))?;
                build(&node, &NoRuntime).map_err(|e| EditError::Build {
                    edit: self.id.clone(),
                    message: e.to_string(),
                })?
            }
            EditBody::Handler(name) => {
                let handler = handlers
                    .get(name.as_str())
                    .ok_or_else(|| EditError::UnknownHandler {
                        edit: self.id.clone(),
                        handler: name.clone(),
                    })?;
                let walks = handler(doc, geometry, &elements).map_err(|message| EditError::Handler {
                    edit: self.id.clone(),
                    message,
                })?;
                return Ok(walks
                    .into_iter()
                    .map(|(mutator, anchors)| DocWalk::new(mutator, anchors))
                    .collect());
            }
        };
        Ok(vec![DocWalk::new(mutator, Anchors::Nodes(elements))])
    }
}

/// How an edits file spells `dto`: the name its `serde` form writes. One
/// spelling, the file's, for every place that names a reach, a target,
/// an easing or a snap to a person. O(1).
fn spelled<D: serde::Serialize>(dto: D) -> String {
    match serde_json::to_value(dto) {
        Ok(serde_json::Value::String(name)) => name,
        other => format!("{other:?}"),
    }
}

impl Reach {
    /// As an edits file spells it: `self`, `children`, … O(1).
    pub fn spelling(self) -> String {
        spelled(ReachDto::from(self))
    }
}

impl Target {
    /// As an edits file spells it: `node`, `sections`, … O(1).
    pub fn spelling(self) -> String {
        spelled(TargetDto::from(self))
    }
}

impl Easing {
    /// As an edits file spells it: `linear`, `ease_in`, … O(1).
    pub fn spelling(self) -> String {
        spelled(EasingDto::from(self))
    }
}

impl Snap {
    /// As an edit's `animate.snap` spells it: `start` or `end`. O(1).
    pub fn spelling(self) -> String {
        spelled(SnapDto::from(self))
    }
}

impl From<ReachDto> for Reach {
    fn from(dto: ReachDto) -> Self {
        match dto {
            ReachDto::SelfNode => Reach::SelfNode,
            ReachDto::Children => Reach::Children,
            ReachDto::Descendants => Reach::Descendants,
            ReachDto::Subtree => Reach::Subtree,
            ReachDto::Parent => Reach::Parent,
            ReachDto::Siblings => Reach::Siblings,
        }
    }
}

impl From<Reach> for ReachDto {
    fn from(reach: Reach) -> Self {
        match reach {
            Reach::SelfNode => ReachDto::SelfNode,
            Reach::Children => ReachDto::Children,
            Reach::Descendants => ReachDto::Descendants,
            Reach::Subtree => ReachDto::Subtree,
            Reach::Parent => ReachDto::Parent,
            Reach::Siblings => ReachDto::Siblings,
        }
    }
}

impl From<TargetDto> for Target {
    fn from(dto: TargetDto) -> Self {
        match dto {
            TargetDto::Node => Target::Node,
            TargetDto::Sections => Target::Sections,
            TargetDto::Spans => Target::Spans,
            TargetDto::Frame => Target::Frame,
            TargetDto::Rings => Target::Rings,
            TargetDto::Connector => Target::Connector,
            TargetDto::Links => Target::Links,
            TargetDto::Labels => Target::Labels,
        }
    }
}

impl From<Target> for TargetDto {
    fn from(target: Target) -> Self {
        match target {
            Target::Node => TargetDto::Node,
            Target::Sections => TargetDto::Sections,
            Target::Spans => TargetDto::Spans,
            Target::Frame => TargetDto::Frame,
            Target::Rings => TargetDto::Rings,
            Target::Connector => TargetDto::Connector,
            Target::Links => TargetDto::Links,
            Target::Labels => TargetDto::Labels,
        }
    }
}

impl From<EasingDto> for Easing {
    fn from(dto: EasingDto) -> Self {
        match dto {
            EasingDto::Linear => Easing::Linear,
            EasingDto::EaseIn => Easing::EaseIn,
            EasingDto::EaseOut => Easing::EaseOut,
            EasingDto::EaseInOut => Easing::EaseInOut,
        }
    }
}

impl From<Easing> for EasingDto {
    fn from(easing: Easing) -> Self {
        match easing {
            Easing::Linear => EasingDto::Linear,
            Easing::EaseIn => EasingDto::EaseIn,
            Easing::EaseOut => EasingDto::EaseOut,
            Easing::EaseInOut => EasingDto::EaseInOut,
        }
    }
}

impl From<SnapDto> for Snap {
    fn from(dto: SnapDto) -> Self {
        match dto {
            SnapDto::Start => Snap::Start,
            SnapDto::End => Snap::End,
        }
    }
}

impl From<Snap> for SnapDto {
    fn from(snap: Snap) -> Self {
        match snap {
            Snap::Start => SnapDto::Start,
            Snap::End => SnapDto::End,
        }
    }
}
