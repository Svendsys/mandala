// SPDX-License-Identifier: MPL-2.0

//! An `f32` as the number a reader expects to see: the shortest
//! decimal that names it (`0.1`, not `0.10000000149011612`).
//!
//! JSON numbers are `f64`, and widening an `f32` to `f64` keeps its
//! exact binary value, whose decimal expansion is long. A serializer
//! writing straight to text prints an `f32` the short way, but one
//! building a `serde_json::Value` first widens it — so the same data
//! read inline and read from a streamed file would differ in every
//! float. [`shortest_f64`](crate::util::shortest_float::shortest_f64) closes that gap at the value: it is the
//! `f64` nearest the `f32`'s shortest decimal, so both paths print the
//! same digits and both parse back to the same `f32`.

use std::fmt::Write;

use serde::Serializer;

/// The `f64` nearest `value`'s shortest round-tripping decimal, or
/// `None` for a non-finite value, which JSON cannot carry.
///
/// # Costs
///
/// One `Display` of the `f32` into a stack buffer and one `f64` parse.
/// No allocation.
pub fn shortest_f64(value: f32) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let mut digits = StackText::default();
    // An `f32`'s shortest `Display` is at most 46 bytes (the subnormal
    // `-0.000…0001` form), which the buffer holds; a write that would
    // not fit fails, and the widened value is the honest fallback.
    if write!(digits, "{value}").is_err() {
        return Some(f64::from(value));
    }
    digits.as_str().parse().ok()
}

/// `#[serde(serialize_with = "serialize_shortest")]` for an `f32`
/// field: [`shortest_f64`], or `null` when it is not finite.
pub fn serialize_shortest<S: Serializer>(value: &f32, serializer: S) -> Result<S::Ok, S::Error> {
    match shortest_f64(*value) {
        Some(shortest) => serializer.serialize_f64(shortest),
        None => serializer.serialize_none(),
    }
}

/// A fixed-capacity `fmt::Write` target, so formatting a float needs
/// no heap.
struct StackText {
    bytes: [u8; 64],
    len: usize,
}

impl Default for StackText {
    fn default() -> Self {
        StackText {
            bytes: [0; 64],
            len: 0,
        }
    }
}

impl StackText {
    fn as_str(&self) -> &str {
        // Only `write_str` fills the buffer, and it copies whole `&str`s.
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}

impl Write for StackText {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len + s.len();
        let slot = self.bytes.get_mut(self.len..end).ok_or(std::fmt::Error)?;
        slot.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}
