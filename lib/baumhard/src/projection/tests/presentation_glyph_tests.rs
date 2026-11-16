// SPDX-License-Identifier: MPL-2.0

//! A glyph-layer presentation's body as run mutations
//! ([`glyph_mutations`]): each op a presentations file can spell, read
//! from the file and turned into the run mutation of the same name, in
//! order.

use crate::document::presentation::{parse_presentations, PresentationBody};
use crate::document::values::Rgba;
use crate::gfx_structs::mutator::GfxMutation;
use crate::gfx_structs::run::RunOverrideOp;
use crate::projection::glyph_mutations;
use glam::Vec2;

/// Every op of the format, each with values that tell its fields apart
/// — an origin's `x` from its `y`, a range's start from its end, a color
/// from no color — becomes the run mutation it names, in the order
/// written. The expected list is written out here from the format's
/// table (`format/presentations.md`), not built by the code under test.
///
/// Controls, each seen red: an origin read `[y, x]`; a range that ends
/// one glyph past its `end`; a color of `null` read as transparent
/// black; the ops reversed.
#[test]
fn test_each_glyph_op_is_the_run_mutation_it_names() {
    let file = r##"{"presentations":[{"id":"g","name":"G","play":"hold","layer":"glyph","target":"sections","glyph":[
{"run_opacity":0.25},
{"run_origin":[3.0,-4.0]},
{"run_overrides":{"offset":{"range":[1,3],"value":[0.5,-2.0]}}},
{"run_overrides":{"color":{"range":[0,2],"value":"#11223344"}}},
{"run_overrides":{"color":{"range":[2,5],"value":null}}},
{"run_overrides":{"scale":{"range":[4,6],"value":1.5}}},
{"run_overrides":"clear"}
]}]}"##;
    let defs = parse_presentations(file).unwrap();
    let PresentationBody::Glyph(ops) = &defs[0].body else {
        panic!("a glyph-layer body: {:?}", defs[0].body);
    };
    assert_eq!(
        glyph_mutations(ops),
        vec![
            GfxMutation::RunOpacity(0.25),
            GfxMutation::RunOrigin(Vec2::new(3.0, -4.0)),
            GfxMutation::RunOverrides(RunOverrideOp::Offset {
                range: 1..3,
                value: Vec2::new(0.5, -2.0),
            }),
            GfxMutation::RunOverrides(RunOverrideOp::Color {
                range: 0..2,
                value: Some(Rgba([0x11, 0x22, 0x33, 0x44])),
            }),
            GfxMutation::RunOverrides(RunOverrideOp::Color {
                range: 2..5,
                value: None,
            }),
            GfxMutation::RunOverrides(RunOverrideOp::Scale {
                range: 4..6,
                value: 1.5,
            }),
            GfxMutation::RunOverrides(RunOverrideOp::Clear),
        ]
    );
}
