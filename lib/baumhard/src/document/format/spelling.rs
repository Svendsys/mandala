// SPDX-License-Identifier: MPL-2.0

//! The file's spellings of the values that are not plain JSON numbers,
//! strings or booleans: a color, a weight, a fit mode, a start point,
//! and an object whose keys the author chooses.
//!
//! Each is a type of its own with a hand-written `Deserialize` and
//! `Serialize`, because each has more than one JSON shape (a color is
//! always a string, a weight a string or a number, a fit mode a string
//! or an object) and `#[serde(untagged)]`, which would express that,
//! hides an unknown key from `deny_unknown_fields` — the strictness
//! gate refuses it. They hold **names, never indices**: `var(accent)` is
//! `ColorDto::Var("accent")`, and only the loader, which has read the
//! theme, turns the name into a
//! [`VarId`](crate::document::values::VarId). Indices are assigned by
//! sorted name, so a saved index would silently name another variable
//! the moment a variable was added; nothing in this file's types can
//! hold one.
//!
//! These are also the spellings a saved predicate uses (design §10.1,
//! V1-D1): a predicate's value is written the way the rest of the file
//! writes it.

use serde::de::{self, IgnoredAny, MapAccess, Unexpected, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

/// Deserialize a present value as `Some`, so that an optional key
/// cannot be written `null`: absence is the only way to say "not set".
/// Used as `#[serde(default, deserialize_with = "present")]`; `default`
/// supplies `None` when the key is absent.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// An object whose keys the author chooses: theme variants, theme
/// variables, palettes. Entries are kept in the order the file gives
/// them, repeated keys included, so that the loader can name the path of
/// a repeat and sort the entries itself; the saver writes them in the
/// order it is given, which is ascending.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Named<T>(pub Vec<(String, T)>);

impl<T> Named<T> {
    /// No entries. For `skip_serializing_if`.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<T> Default for Named<T> {
    fn default() -> Self {
        Named(Vec::new())
    }
}

impl<T: Serialize> Serialize for Named<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (name, value) in &self.0 {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Named<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NamedVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for NamedVisitor<T> {
            type Value = Named<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Named<T>, A::Error> {
                // A hostile hint must not size an allocation.
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0).min(4096));
                while let Some(entry) = map.next_entry::<String, T>()? {
                    entries.push(entry);
                }
                Ok(Named(entries))
            }
        }

        deserializer.deserialize_map(NamedVisitor(PhantomData))
    }
}

/// Why a text is not the canonical spelling of a color.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum HexRefusal {
    /// Not `#` and six or eight hex digits.
    Malformed,
    /// A color, but with an uppercase digit.
    Uppercase([u8; 4]),
    /// A color, but with the alpha `ff` an opaque color leaves out.
    OpaqueAlpha([u8; 4]),
}

impl HexRefusal {
    /// What to tell the author: names the canonical spelling.
    pub(crate) fn message(self, text: &str) -> String {
        match self {
            HexRefusal::Malformed => {
                format!("invalid value: string {text:?}, expected a color, `#rrggbb` or `#rrggbbaa`")
            }
            HexRefusal::Uppercase(color) => format!(
                "{text:?} is not the canonical spelling: hex digits are lowercase, write {:?}",
                format_hex(color)
            ),
            HexRefusal::OpaqueAlpha(color) => format!(
                "{text:?} is not the canonical spelling: an opaque color has no alpha digits, write {:?}",
                format_hex(color)
            ),
        }
    }
}

/// `#rrggbb` or `#rrggbbaa` as four bytes, in the one spelling the file
/// has for each color: lowercase digits, and no alpha digits when the
/// color is opaque (`#rrggbbff` is refused). Anything else is a
/// [`HexRefusal`]. An absent alpha is opaque. O(1).
pub(crate) fn parse_hex(text: &str) -> Result<[u8; 4], HexRefusal> {
    let digits = text.strip_prefix('#').ok_or(HexRefusal::Malformed)?;
    if !matches!(digits.len(), 6 | 8) || !digits.is_ascii() {
        return Err(HexRefusal::Malformed);
    }
    let nibble = |byte: u8| {
        char::from(byte)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
            .ok_or(HexRefusal::Malformed)
    };
    let mut out = [0, 0, 0, 255];
    let (pairs, _) = digits.as_bytes().as_chunks::<2>();
    for (slot, [high, low]) in out.iter_mut().zip(pairs) {
        *slot = (nibble(*high)? << 4) | nibble(*low)?;
    }
    if digits.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(HexRefusal::Uppercase(out));
    }
    if digits.len() == 8 && out[3] == 255 {
        return Err(HexRefusal::OpaqueAlpha(out));
    }
    Ok(out)
}

/// The canonical spelling of a color: lowercase, `#rrggbb` when opaque
/// and `#rrggbbaa` otherwise. O(1).
pub(crate) fn format_hex(color: [u8; 4]) -> String {
    let [r, g, b, a] = color;
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// A literal color: `"#rrggbb"` or `"#rrggbbaa"`. The only spelling a
/// theme variable's value takes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct HexDto(pub [u8; 4]);

impl Serialize for HexDto {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format_hex(self.0))
    }
}

impl<'de> Deserialize<'de> for HexDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct HexVisitor;

        impl Visitor<'_> for HexVisitor {
            type Value = HexDto;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a color, `#rrggbb` or `#rrggbbaa`")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<HexDto, E> {
                parse_hex(text)
                    .map(HexDto)
                    .map_err(|refusal| E::custom(refusal.message(text)))
            }
        }

        deserializer.deserialize_str(HexVisitor)
    }
}

/// Which of a palette group's four colors `palette(...)` names.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum PaletteFieldDto {
    /// `palette(fill)`.
    Fill,
    /// `palette(frame)`.
    Frame,
    /// `palette(text)`.
    Text,
    /// `palette(title)`.
    Title,
}

impl PaletteFieldDto {
    /// The name inside `palette(...)`.
    fn name(self) -> &'static str {
        match self {
            PaletteFieldDto::Fill => "fill",
            PaletteFieldDto::Frame => "frame",
            PaletteFieldDto::Text => "text",
            PaletteFieldDto::Title => "title",
        }
    }

    /// The field a name inside `palette(...)` spells.
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "fill" => PaletteFieldDto::Fill,
            "frame" => PaletteFieldDto::Frame,
            "text" => PaletteFieldDto::Text,
            "title" => PaletteFieldDto::Title,
            _ => return None,
        })
    }
}

/// A value the file writes as one of a table of names. Implemented by
/// `spelled!`, which also gives it `Serialize` and `Deserialize`.
pub(crate) trait Spelled: Copy + PartialEq + 'static {
    /// What the reader says it expected, for an error.
    const WHAT: &'static str;
    /// Every value and its name.
    const NAMES: &'static [(&'static str, Self)];
}

/// Write `value` as its name. O(names).
pub(crate) fn serialize_spelled<T: Spelled, S: Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match T::NAMES.iter().find(|(_, named)| named == value) {
        Some((name, _)) => serializer.serialize_str(name),
        None => Err(serde::ser::Error::custom("a value with no name")),
    }
}

/// Read a name, as a string and nothing else: a unit variant written as
/// an object, `{"left": null}`, is not a name. O(names).
pub(crate) fn deserialize_spelled<'de, T: Spelled, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    struct SpelledVisitor<T>(PhantomData<T>);

    impl<T: Spelled> Visitor<'_> for SpelledVisitor<T> {
        type Value = T;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(T::WHAT)
        }

        fn visit_str<E: de::Error>(self, text: &str) -> Result<T, E> {
            T::NAMES
                .iter()
                .find(|(name, _)| *name == text)
                .map(|(_, value)| *value)
                .ok_or_else(|| E::invalid_value(Unexpected::Str(text), &self))
        }
    }

    deserializer.deserialize_str(SpelledVisitor(PhantomData))
}

/// The corner radius of `{ "rectangle": { "radius": 25 } }`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RectangleDto {
    /// The radius, as a percentage of half the shorter side; checked
    /// against `0..=100` by the loader.
    pub radius: u32,
}

/// A node's outline: `"ellipse"` or `{ "rectangle": { "radius": 25 } }`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum ShapeDto {
    /// `"ellipse"`.
    Ellipse,
    /// `{ "rectangle": { "radius": … } }`.
    Rectangle(RectangleDto),
}

impl Serialize for ShapeDto {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ShapeDto::Ellipse => serializer.serialize_str("ellipse"),
            ShapeDto::Rectangle(rectangle) => {
                serializer.serialize_newtype_variant("ShapeDto", 1, "rectangle", rectangle)
            }
        }
    }
}

impl<'de> Deserialize<'de> for ShapeDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ShapeVisitor;

        impl<'de> Visitor<'de> for ShapeVisitor {
            type Value = ShapeDto;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a shape: `ellipse` or `{\"rectangle\": {\"radius\": …}}`")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<ShapeDto, E> {
                match text {
                    "ellipse" => Ok(ShapeDto::Ellipse),
                    _ => Err(E::invalid_value(Unexpected::Str(text), &self)),
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ShapeDto, A::Error> {
                let Some(key) = map.next_key::<String>()? else {
                    return Err(de::Error::invalid_length(0, &self));
                };
                if key != "rectangle" {
                    return Err(de::Error::unknown_variant(&key, &["rectangle"]));
                }
                let shape = ShapeDto::Rectangle(map.next_value()?);
                if map.next_key::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::invalid_length(2, &self));
                }
                Ok(shape)
            }
        }

        deserializer.deserialize_any(ShapeVisitor)
    }
}

/// A color wherever a color may be written: `"#rrggbb"`,
/// `"#rrggbbaa"`, `"var(name)"` or `"palette(fill|frame|text|title)"`.
/// Hex is lowercase, and an opaque color has no alpha digits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ColorDto<'a> {
    /// A literal color.
    Hex(HexDto),
    /// A theme variable, by name: borrowed from the document when the
    /// saver writes it, owned when the loader reads it.
    Var(Cow<'a, str>),
    /// A color of the palette bound to the owning node.
    Palette(PaletteFieldDto),
}

impl Serialize for ColorDto<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ColorDto::Hex(hex) => hex.serialize(serializer),
            ColorDto::Var(name) => serializer.collect_str(&format_args!("var({name})")),
            ColorDto::Palette(field) => serializer.collect_str(&format_args!("palette({})", field.name())),
        }
    }
}

impl<'de> Deserialize<'de> for ColorDto<'_> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ColorVisitor;

        impl Visitor<'_> for ColorVisitor {
            type Value = ColorDto<'static>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a color: `#rrggbb`, `#rrggbbaa`, `var(name)` or `palette(fill|frame|text|title)`",
                )
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<ColorDto<'static>, E> {
                if text.starts_with('#') {
                    return parse_hex(text)
                        .map(|rgba| ColorDto::Hex(HexDto(rgba)))
                        .map_err(|refusal| E::custom(refusal.message(text)));
                }
                let inner = |prefix: &str| text.strip_prefix(prefix).and_then(|rest| rest.strip_suffix(')'));
                if let Some(name) = inner("var(") {
                    return Ok(ColorDto::Var(Cow::Owned(name.to_string())));
                }
                if let Some(field) = inner("palette(").and_then(PaletteFieldDto::from_name) {
                    return Ok(ColorDto::Palette(field));
                }
                Err(E::invalid_value(Unexpected::Str(text), &self))
            }
        }

        deserializer.deserialize_str(ColorVisitor)
    }
}

/// A font weight: `"normal"` (400), `"bold"` (700), or a number. The
/// number is checked against
/// [`bounds::check_weight`](crate::document::bounds::check_weight) by
/// the loader, so a weight that is not one of the nine is a bound error
/// naming the path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct WeightDto(pub u16);

impl Serialize for WeightDto {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            400 => serializer.serialize_str("normal"),
            700 => serializer.serialize_str("bold"),
            other => serializer.serialize_u16(other),
        }
    }
}

impl<'de> Deserialize<'de> for WeightDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WeightVisitor;

        impl Visitor<'_> for WeightVisitor {
            type Value = WeightDto;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a weight: `normal`, `bold`, or a number from 100 to 900 in steps of 100")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<WeightDto, E> {
                match text {
                    "normal" => Ok(WeightDto(400)),
                    "bold" => Ok(WeightDto(700)),
                    _ => Err(E::invalid_value(Unexpected::Str(text), &self)),
                }
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<WeightDto, E> {
                u16::try_from(value)
                    .map(WeightDto)
                    .map_err(|_| E::invalid_value(Unexpected::Unsigned(value), &self))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<WeightDto, E> {
                match u64::try_from(value) {
                    Ok(value) => self.visit_u64(value),
                    Err(_) => Err(E::invalid_value(Unexpected::Signed(value), &self)),
                }
            }
        }

        deserializer.deserialize_any(WeightVisitor)
    }
}

/// The text a fit mode `"once"` repeats after its content:
/// `{ "once": { "filler": "·" } }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FitOnceDto<'a> {
    /// The filler.
    pub filler: Cow<'a, str>,
}

/// The scale range of `{ "fit": { "min": 0.5, "max": 2.0 } }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FitRangeDto {
    /// The smallest scale.
    pub min: f32,
    /// The largest scale.
    pub max: f32,
}

/// A fit mode: `"repeat"`, `"spread"`, `"once"`,
/// `{ "once": { "filler": "·" } }`, `"fit"` (the built-in bounds) or
/// `{ "fit": { "min": 0.5, "max": 2.0 } }`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FitDto<'a> {
    /// `"repeat"`.
    Repeat,
    /// `"spread"`.
    Spread,
    /// `"once"`, or `{ "once": { "filler": … } }`.
    Once(Option<FitOnceDto<'a>>),
    /// `"fit"`, or `{ "fit": { "min": …, "max": … } }`.
    Fit(Option<FitRangeDto>),
}

impl Serialize for FitDto<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            FitDto::Repeat => serializer.serialize_str("repeat"),
            FitDto::Spread => serializer.serialize_str("spread"),
            FitDto::Once(None) => serializer.serialize_str("once"),
            FitDto::Once(Some(once)) => serializer.serialize_newtype_variant("FitDto", 2, "once", once),
            FitDto::Fit(None) => serializer.serialize_str("fit"),
            FitDto::Fit(Some(range)) => serializer.serialize_newtype_variant("FitDto", 3, "fit", range),
        }
    }
}

impl<'de> Deserialize<'de> for FitDto<'_> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FitVisitor;

        impl<'de> Visitor<'de> for FitVisitor {
            type Value = FitDto<'static>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a fit mode: `repeat`, `spread`, `once`, `fit`, \
                     `{\"once\": {\"filler\": …}}` or `{\"fit\": {\"min\": …, \"max\": …}}`",
                )
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<FitDto<'static>, E> {
                match text {
                    "repeat" => Ok(FitDto::Repeat),
                    "spread" => Ok(FitDto::Spread),
                    "once" => Ok(FitDto::Once(None)),
                    "fit" => Ok(FitDto::Fit(None)),
                    _ => Err(E::invalid_value(Unexpected::Str(text), &self)),
                }
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<FitDto<'static>, A::Error> {
                let Some(key) = map.next_key::<String>()? else {
                    return Err(de::Error::invalid_length(0, &self));
                };
                let fit = match key.as_str() {
                    "once" => FitDto::Once(Some(map.next_value()?)),
                    "fit" => FitDto::Fit(Some(map.next_value()?)),
                    _ => return Err(de::Error::unknown_variant(&key, &["once", "fit"])),
                };
                if map.next_key::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::invalid_length(2, &self));
                }
                Ok(fit)
            }
        }

        deserializer.deserialize_any(FitVisitor)
    }
}

/// A side of a node's outline, as `{ "edge": … }` spells it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum EdgeDto {
    /// `"top"`.
    Top,
    /// `"right"`.
    Right,
    /// `"bottom"`.
    Bottom,
    /// `"left"`.
    Left,
}

spelled!(
    EdgeDto,
    "a side: `top`, `right`, `bottom` or `left`",
    [
        ("top", EdgeDto::Top),
        ("right", EdgeDto::Right),
        ("bottom", EdgeDto::Bottom),
        ("left", EdgeDto::Left),
    ]
);

/// `{ "edge": "top", "at": 0.25 }`: a side and a fraction along it.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartPointDto {
    /// The side.
    pub edge: EdgeDto,
    /// How far along it, measured clockwise.
    pub at: f32,
}

/// The eight named start points, and the side and fraction each means.
const NAMED_STARTS: [(&str, EdgeDto, f32); 8] = [
    ("top-left", EdgeDto::Top, 0.0),
    ("top", EdgeDto::Top, 0.5),
    ("top-right", EdgeDto::Right, 0.0),
    ("right", EdgeDto::Right, 0.5),
    ("bottom-right", EdgeDto::Bottom, 0.0),
    ("bottom", EdgeDto::Bottom, 0.5),
    ("bottom-left", EdgeDto::Left, 0.0),
    ("left", EdgeDto::Left, 0.5),
];

/// Where a frame's text starts: one of eight names, or a side and a
/// fraction. The saver writes the name whenever one matches exactly.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct StartDto(pub StartPointDto);

impl Serialize for StartDto {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let StartPointDto { edge, at } = self.0;
        match NAMED_STARTS
            .iter()
            .find(|(_, named_edge, named_at)| *named_edge == edge && *named_at == at)
        {
            Some((name, _, _)) => serializer.serialize_str(name),
            None => self.0.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for StartDto {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StartVisitor;

        impl<'de> Visitor<'de> for StartVisitor {
            type Value = StartDto;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a start point: `top-left`, `top`, `top-right`, `right`, `bottom-right`, \
                     `bottom`, `bottom-left`, `left` or `{\"edge\": …, \"at\": …}`",
                )
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<StartDto, E> {
                NAMED_STARTS
                    .iter()
                    .find(|(name, _, _)| *name == text)
                    .map(|&(_, edge, at)| StartDto(StartPointDto { edge, at }))
                    .ok_or_else(|| E::invalid_value(Unexpected::Str(text), &self))
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<StartDto, A::Error> {
                StartPointDto::deserialize(de::value::MapAccessDeserializer::new(map)).map(StartDto)
            }
        }

        deserializer.deserialize_any(StartVisitor)
    }
}
