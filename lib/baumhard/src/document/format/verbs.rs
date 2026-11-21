// SPDX-License-Identifier: MPL-2.0

//! A verb as a file writes it (`format/verbs.md`; design §11.1, D7):
//! its identity and its steps, each step one of the five kinds the
//! format has and nothing else.
//!
//! **Names, never indices.** A step names a presentation, an edit and a
//! verb by the `id` the map gives it, and a node by its `id`; the loader
//! resolves the definitions against the map's own lists
//! (`document::verb::load`).
//!
//! **Strict.** A step is one object with exactly one of the five step
//! keys — `present`, `stop`, `edit`, `wait`, `verb` — and `on` where its
//! kind takes one; any other key is refused, which is the sandbox (design
//! §11.3): a verb has no spelling for a console line, an Action, a file
//! or the process. A target (`on`) is a string — `"self"`,
//! `"selection"`, `"children"`, `"descendants"` — or an object,
//! `{ "nodes": [ids] }` or `{ "channel": n, "within": "self" | "map" }`,
//! read by a reader of its own that takes those spellings and nothing
//! else.

use crate::document::format::spelling::present;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// One verb definition, as a file writes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerbDto {
    /// The id triggers, links, the console and IPC name it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// What it does, for `verb list`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Its steps, in order.
    pub steps: Vec<StepDto>,
}

/// One step: exactly one of the five step keys, and `on` beside
/// `present`, `stop` and `edit`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StepDto {
    /// Start the presentation with this id on the targets.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub present: Option<String>,
    /// Stop the presentation with this id on the targets.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub stop: Option<String>,
    /// Apply the edit with this id at the targets.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub edit: Option<String>,
    /// Wait this many milliseconds on the app clock.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub wait: Option<u32>,
    /// Run the verb with this id, inline.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub verb: Option<String>,
    /// The targets of a `present`, `stop` or `edit` step.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub on: Option<StepTargetDto>,
}

/// A step's targets (`on`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StepTargetDto {
    /// `"self"`: the node the verb was invoked on.
    SelfNode,
    /// `"selection"`: what is selected when the step runs.
    Selection,
    /// `"children"`: the child nodes of `self`.
    Children,
    /// `"descendants"`: every node below `self`.
    Descendants,
    /// `{ "nodes": [ids] }`: these nodes, by id.
    Nodes(Vec<u64>),
    /// `{ "channel": n, "within": "self" | "map" }`: the nodes on
    /// channel `n`, below `self` or anywhere in the map.
    Channel {
        /// The channel.
        channel: u64,
        /// Where.
        within: WithinDto,
    },
}

/// Where a channel target looks: `"self"` or `"map"`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum WithinDto {
    /// `"self"`: the nodes below the verb's `self` node.
    SelfNode,
    /// `"map"`: every node of the map.
    Map,
}

spelled!(
    WithinDto,
    "where a channel target looks: `self` or `map`",
    [("self", WithinDto::SelfNode), ("map", WithinDto::Map)]
);

/// The names a target written as a string takes.
const ON_NAMES: [(&str, StepTargetDto); 4] = [
    ("self", StepTargetDto::SelfNode),
    ("selection", StepTargetDto::Selection),
    ("children", StepTargetDto::Children),
    ("descendants", StepTargetDto::Descendants),
];

/// What a target is, for the reader's refusal.
const ON_WHAT: &str = "a target: `self`, `selection`, `children`, `descendants`, \
                       `{ \"nodes\": [ids] }` or `{ \"channel\": n, \"within\": \"self\" | \"map\" }`";

impl Serialize for StepTargetDto {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            StepTargetDto::Nodes(ids) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("nodes", ids)?;
                map.end()
            }
            StepTargetDto::Channel { channel, within } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("channel", channel)?;
                map.serialize_entry("within", within)?;
                map.end()
            }
            named => match ON_NAMES.iter().find(|(_, value)| value == named) {
                Some((name, _)) => serializer.serialize_str(name),
                None => Err(serde::ser::Error::custom("a target with no name")),
            },
        }
    }
}

impl<'de> Deserialize<'de> for StepTargetDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OnVisitor;

        impl<'de> Visitor<'de> for OnVisitor {
            type Value = StepTargetDto;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(ON_WHAT)
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<StepTargetDto, E> {
                ON_NAMES
                    .iter()
                    .find(|(name, _)| *name == text)
                    .map(|(_, value)| value.clone())
                    .ok_or_else(|| E::invalid_value(de::Unexpected::Str(text), &self))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<StepTargetDto, A::Error> {
                let mut nodes: Option<Vec<u64>> = None;
                let mut channel: Option<u64> = None;
                let mut within: Option<WithinDto> = None;
                while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
                    match key.as_ref() {
                        "nodes" if nodes.is_none() => nodes = Some(map.next_value()?),
                        "channel" if channel.is_none() => channel = Some(map.next_value()?),
                        "within" if within.is_none() => within = Some(map.next_value()?),
                        "nodes" | "channel" | "within" => return Err(de::Error::duplicate_field("on")),
                        other => {
                            return Err(de::Error::unknown_field(other, &["nodes", "channel", "within"]));
                        }
                    }
                }
                match (nodes, channel, within) {
                    (Some(ids), None, None) => Ok(StepTargetDto::Nodes(ids)),
                    (None, Some(channel), Some(within)) => Ok(StepTargetDto::Channel { channel, within }),
                    (None, Some(_), None) => Err(de::Error::missing_field("within")),
                    (None, None, Some(_)) => Err(de::Error::missing_field("channel")),
                    _ => Err(de::Error::invalid_value(de::Unexpected::Map, &self)),
                }
            }
        }

        deserializer.deserialize_any(OnVisitor)
    }
}
