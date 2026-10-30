// SPDX-License-Identifier: MPL-2.0

//! The positioned-glyph pass's decisions, pinned without a device
//! (TEST_CONVENTIONS §T8's renderer-free tier): where each glyph of a
//! run lands, which rasterization it asks for, and how overrides,
//! opacity, zoom and hostile values reach it. The GPU half — the atlas,
//! the pipeline, one draw — is pinned by `app::tests_gpu_glyph_runs`.
//!
//! Runs are shaped for real in a compiled-in face, so a glyph's id and
//! offsets are the shaper's; expected positions are worked from the
//! placement the test chose, the camera it built and, where the subject
//! is "as cosmic-text places it", from cosmic-text's own
//! `LayoutGlyph::physical` (§T13 shape 6: an oracle that shares no code
//! with the emitter). The font lock is held only while shaping, never
//! across an assertion, so a red test cannot poison it for the others.

use std::sync::Arc;

use baumhard::document::resolve::ResolvedText;
use baumhard::document::values::{FitMode, FontSize, Rgba, Weight, ZoomWindow};
use baumhard::flow::{
    place_at_point, place_on_open, shape_box, shape_run, Curve, FlowSpan, PlacedGlyph, PlacedRun, ShapedRun,
};
use baumhard::font::fonts::{self, acquire_font_system_write, AppFont, FontSystemWriteGuard};
use baumhard::font::LayoutGlyph;
use baumhard::gfx_structs::camera::Camera2D;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutation;
use baumhard::gfx_structs::run::{GlyphOverrides, GlyphRunData, RunOverrideOp, RunShape};
use baumhard::mindmap::custom_mutation::apply_mutations_to_element;
use glam::Vec2;

use super::atlas::{Page, Slot};
use super::emit::{emit_run, EmitCounts, GlyphSprite, MAX_RASTER_FONT_PX};
use super::instance;

/// The face every run here is shaped in: compiled into the binary, so
/// the glyphs do not depend on the machine's fonts.
const FACE: AppFont = AppFont::GalatiaSIL;

const SIZE: f32 = 20.0;

fn style(points: f32, color: Rgba) -> ResolvedText {
    ResolvedText {
        font: FACE,
        size: FontSize::Pt(points),
        color,
        weight: Weight::NORMAL,
        italic: false,
    }
}

const TEXT_COLOR: Rgba = Rgba([200, 100, 50, 255]);

/// Run `f` under the font guard, released before the result returns.
fn with_fonts<R>(f: impl FnOnce(&mut FontSystemWriteGuard) -> R) -> R {
    fonts::init();
    let mut fs = acquire_font_system_write("renderer::glyph_runs::tests");
    f(&mut fs)
}

fn shaped(text: &str) -> ShapedRun {
    shaped_at(text, SIZE)
}

fn shaped_at(text: &str, size: f32) -> ShapedRun {
    let style = style(size, TEXT_COLOR);
    with_fonts(|fs| {
        shape_run(
            &[FlowSpan {
                text,
                style: &style,
                link: None,
            }],
            1.0,
            fs,
        )
    })
}

/// A camera whose canvas units are surface pixels: zoom 1, centered so
/// that canvas `(x, y)` is surface pixel `(x, y)`.
fn identity_camera() -> Camera2D {
    let mut camera = Camera2D::new(800, 600);
    camera.position = Vec2::new(400.0, 300.0);
    camera
}

fn path_run(run: ShapedRun, placed: Vec<PlacedGlyph>, origin: Vec2) -> GlyphRunData {
    GlyphRunData::new(
        RunShape::Path {
            run: Arc::new(run),
            placed: Arc::new(PlacedRun {
                glyphs: placed,
                truncated: false,
            }),
        },
        origin,
        GlyphOverrides::none(),
        1.0,
        None,
    )
    .expect("no overrides, so no length to get wrong")
}

fn at(glyph: u32, x: f32, y: f32) -> PlacedGlyph {
    PlacedGlyph {
        glyph,
        pos: Vec2::new(x, y),
    }
}

/// Every sprite `run` emits through `camera`, and the counts.
fn emit(run: &GlyphRunData, camera: &Camera2D) -> (Vec<GlyphSprite>, EmitCounts) {
    emit_at(run, Vec2::ZERO, camera)
}

fn emit_at(run: &GlyphRunData, offset: Vec2, camera: &Camera2D) -> (Vec<GlyphSprite>, EmitCounts) {
    let mut sprites = Vec::new();
    let mut layout: Vec<&LayoutGlyph> = Vec::new();
    let counts = emit_run(run, offset, camera, &mut layout, &mut |sprite| {
        sprites.push(sprite)
    });
    (sprites, counts)
}

/// `run` with `ops` applied through the glyph mutation path a
/// presentation writes overrides through.
fn with_overrides(run: GlyphRunData, ops: &[RunOverrideOp]) -> GlyphRunData {
    let mut element = GfxElement::new_run(run, 0, 1);
    let mutations: Vec<GfxMutation> = ops.iter().cloned().map(GfxMutation::RunOverrides).collect();
    apply_mutations_to_element(&mutations, &mut element);
    element.glyph_run().cloned().expect("still a run")
}

fn layout_of(run: &GlyphRunData) -> Vec<LayoutGlyph> {
    match run.shaped() {
        RunShape::Path { run, .. } => run.layout_glyphs().cloned().collect(),
        RunShape::Box(_) => panic!("a path run"),
    }
}

fn baseline_of(run: &GlyphRunData) -> f32 {
    match run.shaped() {
        RunShape::Path { run, .. } => run.baseline(),
        RunShape::Box(_) => panic!("a path run"),
    }
}

/// **Glyph *i* of a placement is drawn at its placed position, on the
/// run's baseline, as the glyph the placement names.** Placed glyph 0
/// names shaped glyph 1 and placed glyph 1 names shaped glyph 0, so an
/// emitter that took the *i*-th laid-out glyph for placed glyph *i*
/// would draw `"BA"` where the placement says `"AB"`. The pen is the
/// box's top-left plus the baseline, offset by the run's origin; the
/// key carries the glyph's face, id and size.
#[test]
fn test_glyph_run_emit_draws_each_placed_glyph_at_its_position_on_the_baseline() {
    let run = path_run(
        shaped("AB"),
        vec![at(1, 100.0, 50.0), at(0, 300.25, 70.0)],
        Vec2::new(10.0, 20.0),
    );
    let layout = layout_of(&run);
    let baseline = baseline_of(&run);
    assert!(
        baseline > 1.0 && baseline < SIZE * 1.2 - 1.0,
        "precondition: the baseline sits inside the box ({baseline}), so dropping it is visible"
    );
    assert_ne!(
        layout[0].glyph_id, layout[1].glyph_id,
        "precondition: the two glyphs differ, so a swapped index is visible"
    );
    assert!(
        layout.iter().all(|g| g.x_offset == 0.0 && g.y_offset == 0.0),
        "precondition: Latin capitals carry no shaper offsets here"
    );

    let (sprites, counts) = emit(&run, &identity_camera());
    assert_eq!(counts.sprites, 2);
    assert!(counts.drawn);
    assert_eq!(
        sprites[0].key.glyph_id, layout[1].glyph_id,
        "placed glyph 0 names shaped glyph 1"
    );
    assert_eq!(
        sprites[1].key.glyph_id, layout[0].glyph_id,
        "placed glyph 1 names shaped glyph 0"
    );
    assert_eq!(sprites[0].key.font_id, layout[1].font_id);
    assert_eq!(
        f32::from_bits(sprites[0].key.font_size_bits),
        SIZE,
        "rasterized at its size, zoom 1"
    );
    assert_eq!(
        sprites[0].pen,
        (110, (70.0 + baseline).floor() as i32),
        "box top-left (100, 50) + origin (10, 20), baseline below"
    );
    assert_eq!(
        sprites[1].pen,
        (310, (90.0 + baseline).floor() as i32),
        "x = 310.25: the quarter pixel goes into the key's subpixel bin"
    );
    assert_eq!(
        sprites[1].key.x_bin,
        glyphon::SubpixelBin::One,
        "0.25 px is bin One"
    );
    assert_eq!(sprites[0].key.x_bin, glyphon::SubpixelBin::Zero);
    assert_eq!(
        sprites[0].color, TEXT_COLOR.0,
        "a glyph keeps the color it was shaped with"
    );
}

/// **The camera, and only the camera, decides surface pixels and the
/// rasterized size: zoom applies at render time.** At zoom 2 around
/// canvas `(400, 300)` the same run draws at twice the distance from
/// the surface center and is rasterized at twice the size; the run
/// itself is not touched. A tree offset moves every glyph with it.
#[test]
fn test_glyph_run_emit_applies_the_camera_and_the_tree_offset_at_render_time() {
    let run = path_run(shaped("A"), vec![at(0, 410.0, 300.0)], Vec2::ZERO);
    let baseline = baseline_of(&run);
    let mut camera = identity_camera();
    camera.zoom = 2.0;
    let (sprites, _) = emit(&run, &camera);
    assert_eq!(
        f32::from_bits(sprites[0].key.font_size_bits),
        SIZE * 2.0,
        "rasterized at the zoomed size"
    );
    assert_eq!(
        sprites[0].pen,
        (420, (300.0 + 2.0 * baseline).floor() as i32),
        "(410 − 400) × 2 + 400 across; (300 + baseline − 300) × 2 + 300 down"
    );

    let (moved, _) = emit_at(&run, Vec2::new(5.0, 0.0), &camera);
    assert_eq!(
        moved[0].pen.0, 430,
        "a tree offset of 5 canvas units is 10 pixels at zoom 2"
    );
    assert_eq!(
        moved[0].key, sprites[0].key,
        "and asks for the same rasterization"
    );
}

/// **A shaper offset is kept: placement replaces a glyph's pen, not its
/// offsets.** A combining acute on `x` is positioned by the shaper
/// relative to its base; drawn at a placed position, it must land
/// `font size × x_offset` right and `font size × y_offset` up of that
/// pen, as cosmic-text's `LayoutGlyph::physical` puts it. At 100 pt
/// and the camera's largest zoom both offsets are whole pixels — the
/// vertical one more than three, so a `y_offset` dropped or added with
/// the wrong sign moves the pen past the one-pixel tolerance the
/// truncation needs — and dropping either moves the pen, not only the
/// key's subpixel bin.
#[test]
fn test_glyph_run_emit_keeps_each_glyphs_shaper_offsets() {
    const MARK_SIZE: f32 = 100.0;
    let run = shaped_at("x\u{301}", MARK_SIZE);
    let layout: Vec<LayoutGlyph> = run.layout_glyphs().cloned().collect();
    let Some(mark) = layout.iter().position(|g| g.x_offset != 0.0 || g.y_offset != 0.0) else {
        panic!("precondition: the shaper positioned the mark with an offset: {layout:?}")
    };
    let laid = layout[mark].clone();
    let zoom = Camera2D::MAX_ZOOM;
    assert!(
        (laid.x_offset * MARK_SIZE * zoom).abs() >= 1.0,
        "precondition: the mark's x offset is at least a pixel at zoom {zoom}: {laid:?}"
    );
    assert!(
        (laid.y_offset * MARK_SIZE * zoom).abs() >= 3.0,
        "precondition: the mark's y offset is at least three pixels at zoom {zoom}, past the \
         one-pixel tolerance below: {laid:?}"
    );
    let run = path_run(run, vec![at(mark as u32, 100.0, 100.0)], Vec2::ZERO);
    let baseline = baseline_of(&run);
    let mut camera = identity_camera();
    camera.position = Vec2::new(100.0, 100.0);
    camera.zoom = zoom;
    let (sprites, _) = emit(&run, &camera);

    // cosmic-text's own placement of the glyph on a line whose pen is
    // where the placement put it — surface (400, 300 + zoom × baseline)
    // — is the oracle.
    let mut oracle = laid.clone();
    oracle.x = 0.0;
    oracle.y = 0.0;
    let physical = oracle.physical((400.0, 300.0 + zoom * baseline), zoom);
    assert_eq!(
        sprites[0].key, physical.cache_key,
        "the same rasterization, bins included"
    );
    assert_eq!(
        sprites[0].pen.0, physical.x,
        "x_offset applied as physical applies it"
    );
    assert!(
        (sprites[0].pen.1 - physical.y).abs() <= 1,
        "y_offset applied as physical applies it, to the pixel its truncation picks: {} vs {}",
        sprites[0].pen.1,
        physical.y
    );
    assert_ne!(
        sprites[0].pen.0, 400,
        "control: the mark does not sit on the bare pen"
    );
}

/// The pen of a sprite on the surface, its subpixel bin included.
fn pen_x(sprite: &GlyphSprite) -> f32 {
    sprite.pen.0 as f32 + sprite.key.x_bin.as_float()
}

/// **A scale override scales a grapheme cluster about its own box, so a
/// combining mark stays on its base.** `"x\u{301}"` placed by the real
/// `place_at_point` (a path run) and shaped as a box: scaled ×3, the
/// mark's pen sits three times as far from the base's pen as it does
/// unscaled — where the cluster scaled whole puts it. Scaling each
/// glyph about its own advance box would leave a zero-advance mark at
/// twice the distance, a third of the way into the tripled base.
#[test]
fn test_glyph_run_emit_scales_a_cluster_about_its_own_box() {
    const K: f32 = 3.0;
    let run = shaped("x\u{301}");
    assert_eq!(run.glyphs.len(), 2, "precondition: a base and a mark glyph");
    assert!(
        run.glyphs[0].grapheme_start && !run.glyphs[1].grapheme_start,
        "precondition: the two glyphs are one grapheme cluster"
    );
    let placed = place_at_point(&run, Vec2::new(400.0, 300.0));
    let path = GlyphRunData::new(
        RunShape::Path {
            run: Arc::new(run),
            placed: Arc::new(placed),
        },
        Vec2::ZERO,
        GlyphOverrides::none(),
        1.0,
        None,
    )
    .expect("no overrides");

    let style = style(SIZE, TEXT_COLOR);
    let boxed = with_fonts(|fs| {
        shape_box(
            &[FlowSpan {
                text: "x\u{301}",
                style: &style,
                link: None,
            }],
            None,
            baumhard::document::values::Align::Left,
            fs,
        )
    });
    assert_eq!(
        boxed.glyph_spans[0], boxed.glyph_spans[1],
        "precondition: the box's two glyphs share one span byte range"
    );
    let boxed = GlyphRunData::new(
        RunShape::Box(Arc::new(boxed)),
        Vec2::new(380.0, 280.0),
        GlyphOverrides::none(),
        1.0,
        None,
    )
    .expect("no overrides");

    for (what, run) in [("path", path), ("box", boxed)] {
        let (plain, _) = emit(&run, &identity_camera());
        let unscaled = pen_x(&plain[1]) - pen_x(&plain[0]);
        assert!(
            unscaled.abs() >= 2.0,
            "{what}: precondition: the mark sits apart from its base's pen ({unscaled})"
        );
        let scaled = with_overrides(
            run,
            &[RunOverrideOp::Scale {
                range: 0..2,
                value: K,
            }],
        );
        let (big, _) = emit(&scaled, &identity_camera());
        let delta = pen_x(&big[1]) - pen_x(&big[0]);
        assert!(
            (delta - K * unscaled).abs() <= 0.5,
            "{what}: the mark keeps its place on the tripled base: {delta} vs {} (unscaled {unscaled})",
            K * unscaled
        );
    }
}

/// **A glyph an offset carries onto the surface is drawn, though its
/// run's box is off it.** `"AB"` at x = −200 and −180, offset by
/// (400, 0): both glyphs land near x = 200 on an 800-pixel surface. The
/// run's box — its hit-test box, which overrides do not move — is off
/// screen, so a run culled by it would draw nothing. The control is the
/// same run without the offset, which draws nothing.
#[test]
fn test_glyph_run_emit_draws_glyphs_an_offset_carries_on_screen() {
    let base = path_run(
        shaped("AB"),
        vec![at(0, -200.0, 100.0), at(1, -180.0, 100.0)],
        Vec2::ZERO,
    );
    let (sprites, counts) = emit(&base, &identity_camera());
    assert!(
        sprites.is_empty() && !counts.drawn,
        "control: without the offset the run is off screen"
    );
    let moved = with_overrides(
        base,
        &[RunOverrideOp::Offset {
            range: 0..2,
            value: Vec2::new(400.0, 0.0),
        }],
    );
    let (sprites, counts) = emit(&moved, &identity_camera());
    assert_eq!(counts.sprites, 2, "both offset glyphs are drawn: {counts:?}");
    assert_eq!(
        (sprites[0].pen.0, sprites[1].pen.0),
        (200, 220),
        "where the offset put them"
    );
}

/// **A color glyph takes the run's opacity and nothing else; a mask
/// glyph takes its color, alpha and all.** One sprite — an override
/// color of alpha 0 and opacity 0.5 folded in — drawn from each page:
/// from the mask page it is the color with alpha 0; from the color page
/// (an emoji) the color does not reach it and its alpha is the run's
/// opacity, 0.5.
#[test]
fn test_glyph_run_instance_gives_a_color_glyph_the_opacity_alone() {
    let sprite = GlyphSprite {
        key: shaped("A")
            .layout_glyphs()
            .next()
            .expect("a glyph")
            .physical((0.0, 0.0), 1.0)
            .cache_key,
        pen: (10, 20),
        color: [255, 0, 0, 0],
        opacity: 0.5,
    };
    let slot = |page| Slot {
        page,
        uv: (3, 4),
        size: (5, 6),
        left: 1,
        top: 2,
    };
    let mask = instance(&sprite, slot(Page::Mask));
    assert_eq!(
        mask.color,
        [1.0, 0.0, 0.0, 0.0],
        "a mask glyph takes its color's alpha"
    );
    assert_eq!(mask.page, 1.0);
    let color = instance(&sprite, slot(Page::Color));
    assert_eq!(
        color.color,
        [1.0, 1.0, 1.0, 0.5],
        "an emoji keeps its colors, and its alpha is the run's opacity"
    );
    assert_eq!(color.page, 0.0);
    assert_eq!(
        (color.pos, color.size, color.uv),
        ([11.0, 18.0], [5.0, 6.0], [3.0, 4.0]),
        "the bitmap placed by its bearing from the pen"
    );
}

/// **Overrides and opacity.** On a three-glyph run: glyph 0 is moved by
/// an offset, glyph 1 recolored, glyph 2 doubled about its box's center;
/// the run's opacity halves every alpha. Each override touches its own
/// glyph and no other.
#[test]
fn test_glyph_run_emit_applies_each_override_to_its_own_glyph_and_opacity_to_all() {
    let shaped = shaped("ABC");
    let advance = shaped.glyphs[2].advance;
    let line_height = shaped.line_height;
    let base = path_run(
        shaped,
        vec![at(0, 100.0, 100.0), at(1, 200.0, 100.0), at(2, 300.0, 100.0)],
        Vec2::ZERO,
    );
    let baseline = baseline_of(&base);
    let (plain, _) = emit(&base, &identity_camera());

    let mut run = with_overrides(
        base,
        &[
            RunOverrideOp::Offset {
                range: 0..1,
                value: Vec2::new(7.0, -3.0),
            },
            RunOverrideOp::Color {
                range: 1..2,
                value: Some(Rgba([0, 255, 0, 200])),
            },
            RunOverrideOp::Scale {
                range: 2..3,
                value: 2.0,
            },
        ],
    );
    let mut element = GfxElement::new_run(run, 0, 1);
    apply_mutations_to_element(&[GfxMutation::RunOpacity(0.5)], &mut element);
    run = element.glyph_run().cloned().expect("a run");

    let (sprites, counts) = emit(&run, &identity_camera());
    assert_eq!(counts.sprites, 3);
    assert_eq!(
        sprites[0].pen,
        (plain[0].pen.0 + 7, plain[0].pen.1 - 3),
        "glyph 0 moved by its offset"
    );
    assert_eq!(
        sprites[0].key, plain[0].key,
        "an offset by whole pixels needs no new rasterization"
    );
    assert_eq!(sprites[1].pen, plain[1].pen, "glyph 1 did not move");
    assert_eq!(
        sprites[1].color,
        [0, 255, 0, 100],
        "glyph 1 recolored, its alpha halved by the opacity"
    );
    assert_eq!(
        sprites[0].color,
        [200, 100, 50, 128],
        "opacity halves glyph 0's alpha too"
    );
    assert_eq!(
        f32::from_bits(sprites[2].key.font_size_bits),
        SIZE * 2.0,
        "glyph 2 doubled"
    );
    // Scaled about the center of its box: the pen moves away from it.
    let center = Vec2::new(300.0 + advance / 2.0, 100.0 + line_height / 2.0);
    let pen = center + (Vec2::new(300.0, 100.0 + baseline) - center) * 2.0;
    assert!(
        (sprites[2].pen.0 as f32 - pen.x).abs() <= 1.0,
        "doubled about its center: x {} vs {} (the fraction is the key's bin)",
        sprites[2].pen.0,
        pen.x
    );
    assert_eq!(
        sprites[2].pen.1,
        pen.y.floor() as i32,
        "doubled about its center: y"
    );
    assert_eq!(
        f32::from_bits(sprites[1].key.font_size_bits),
        SIZE,
        "the scale reached glyph 2 only"
    );
}

/// **A filled run is two buffer lines, and a placement indexes both.**
/// `Once` with a filler returns the run plus a filler line; the placed
/// glyphs past the text name the filler's glyph, which is the second
/// line's, and each is drawn as that glyph.
#[test]
fn test_glyph_run_emit_draws_a_filler_from_the_runs_second_line() {
    let style = style(SIZE, TEXT_COLOR);
    let (run, placed) = with_fonts(|fs| {
        let text = shape_run(
            &[FlowSpan {
                text: "AB",
                style: &style,
                link: None,
            }],
            1.0,
            fs,
        );
        let curve = Curve::Line(Vec2::new(0.0, 100.0), Vec2::new(400.0, 100.0));
        let fit = FitMode::Once {
            filler: Some("~".to_string()),
        };
        place_on_open(&text, &curve, &fit, 0.0, &style, fs)
    });
    let run = run.expect("a filler is placed, so the placement refers to a new run");
    assert_eq!(
        run.buffer.lines.len(),
        2,
        "precondition: text and filler on lines of their own"
    );
    let filler_glyph = run.layout_glyphs().nth(2).expect("the filler's glyph").glyph_id;
    let text_ids: Vec<u16> = run.layout_glyphs().take(2).map(|g| g.glyph_id).collect();
    assert!(
        !text_ids.contains(&filler_glyph),
        "precondition: the filler is another glyph"
    );
    assert!(
        placed.glyphs.len() > 4,
        "precondition: the filler repeats along the line"
    );

    let data = GlyphRunData::new(
        RunShape::Path {
            run: Arc::new(run),
            placed: Arc::new(placed.clone()),
        },
        Vec2::ZERO,
        GlyphOverrides::none(),
        1.0,
        None,
    )
    .expect("no overrides");
    let (sprites, counts) = emit(&data, &identity_camera());
    assert_eq!(
        counts.sprites,
        placed.glyphs.len(),
        "every placed glyph drawn once"
    );
    assert_eq!(
        &sprites[..2].iter().map(|s| s.key.glyph_id).collect::<Vec<_>>(),
        &text_ids
    );
    assert!(
        sprites[2..].iter().all(|s| s.key.glyph_id == filler_glyph),
        "every glyph after the text is the filler's"
    );
}

/// **A box run is its buffer drawn whole at its origin**, each glyph as
/// cosmic-text's `LayoutGlyph::physical` places it on its line — the
/// placement glyphon gives a `TextArea` — and the second line one line
/// height below the first.
#[test]
fn test_glyph_run_emit_draws_a_box_whole_at_its_origin() {
    let style = style(SIZE, TEXT_COLOR);
    let shaped = with_fonts(|fs| {
        shape_box(
            &[FlowSpan {
                text: "A\nB",
                style: &style,
                link: None,
            }],
            None,
            baumhard::document::values::Align::Left,
            fs,
        )
    });
    let lines: Vec<(f32, Vec<LayoutGlyph>)> = shaped
        .buffer
        .layout_runs()
        .map(|line| (line.line_y, line.glyphs.to_vec()))
        .collect();
    assert_eq!(lines.len(), 2, "precondition: two lines");
    let run = GlyphRunData::new(
        RunShape::Box(Arc::new(shaped)),
        Vec2::new(50.0, 60.0),
        GlyphOverrides::none(),
        1.0,
        None,
    )
    .expect("no overrides");
    let (sprites, counts) = emit(&run, &identity_camera());
    assert_eq!(counts.sprites, 2);
    for (sprite, (line_y, glyphs)) in sprites.iter().zip(&lines) {
        let physical = glyphs[0].physical((50.0, 60.0), 1.0);
        assert_eq!(
            sprite.key, physical.cache_key,
            "the rasterization physical asks for"
        );
        assert_eq!(sprite.pen.0, physical.x, "x as physical places it");
        let glyphon_y = physical.y + line_y.round() as i32;
        assert!(
            (sprite.pen.1 - glyphon_y).abs() <= 1,
            "on its line's baseline, to the pixel: {} vs {glyphon_y}",
            sprite.pen.1
        );
    }
    assert!(
        sprites[1].pen.1 - sprites[0].pen.1 >= (SIZE * 1.2) as i32 - 1,
        "the second line is a line height lower"
    );
}

/// **What a camera does not show is not drawn, nor rasterized.** A run
/// outside its zoom window, or wholly off screen, emits nothing; a
/// transparent one emits nothing; a glyph far past the surface's edge
/// is skipped while its neighbor on screen is drawn.
#[test]
fn test_glyph_run_emit_culls_what_the_camera_does_not_show() {
    let camera = identity_camera();
    let shaped = shaped("AB");
    let visible = path_run(shaped.clone(), vec![at(0, 100.0, 100.0)], Vec2::ZERO);
    assert_eq!(
        emit(&visible, &camera).1.sprites,
        1,
        "control: on screen and in its window"
    );

    let windowed = GlyphRunData::new(
        visible.shaped().clone(),
        Vec2::ZERO,
        GlyphOverrides::none(),
        1.0,
        Some(ZoomWindow {
            min: Some(1.5),
            max: None,
        }),
    )
    .expect("no overrides");
    let (sprites, counts) = emit(&windowed, &camera);
    assert!(
        sprites.is_empty() && !counts.drawn,
        "below its window's minimum zoom"
    );

    let away = path_run(shaped.clone(), vec![at(0, 5000.0, 100.0)], Vec2::ZERO);
    assert!(!emit(&away, &camera).1.drawn, "wholly off screen");

    let mut element = GfxElement::new_run(visible.clone(), 0, 1);
    apply_mutations_to_element(&[GfxMutation::RunOpacity(0.0)], &mut element);
    let transparent = element.glyph_run().cloned().expect("a run");
    assert!(emit(&transparent, &camera).0.is_empty(), "fully transparent");

    let spread = path_run(
        shaped,
        vec![at(0, 100.0, 100.0), at(1, 3000.0, 100.0)],
        Vec2::ZERO,
    );
    let (sprites, counts) = emit(&spread, &camera);
    assert_eq!(
        (sprites.len(), counts.skipped),
        (1, 1),
        "the far glyph is skipped, its neighbor drawn"
    );
}

/// **Hostile values degrade, never panic, and never ask the rasterizer
/// for an absurd size** (D29: a map authors override values). A scale
/// of `NaN`, `0`, negative, `∞` or past [`MAX_RASTER_FONT_PX`], an
/// offset of `NaN` or `∞`, and an opacity of `NaN`: each glyph so
/// marked is skipped, its neighbors drawn.
#[test]
fn test_glyph_run_emit_skips_hostile_values_and_draws_the_rest() {
    let camera = identity_camera();
    let base = path_run(
        shaped("ABCDEFGH"),
        (0..8).map(|i| at(i, 50.0 + 60.0 * i as f32, 100.0)).collect(),
        Vec2::ZERO,
    );
    let huge = MAX_RASTER_FONT_PX / SIZE * 2.0;
    let run = with_overrides(
        base.clone(),
        &[
            RunOverrideOp::Scale {
                range: 0..1,
                value: f32::NAN,
            },
            RunOverrideOp::Scale {
                range: 1..2,
                value: 0.0,
            },
            RunOverrideOp::Scale {
                range: 2..3,
                value: -1.0,
            },
            RunOverrideOp::Scale {
                range: 3..4,
                value: f32::INFINITY,
            },
            RunOverrideOp::Scale {
                range: 4..5,
                value: huge,
            },
            RunOverrideOp::Offset {
                range: 5..6,
                value: Vec2::new(f32::NAN, 0.0),
            },
            RunOverrideOp::Offset {
                range: 6..7,
                value: Vec2::new(0.0, f32::INFINITY),
            },
        ],
    );
    let (sprites, counts) = emit(&run, &camera);
    assert_eq!(
        (sprites.len(), counts.skipped),
        (1, 7),
        "seven hostile glyphs skipped, the eighth drawn"
    );
    assert!(
        sprites
            .iter()
            .all(|s| f32::from_bits(s.key.font_size_bits) <= MAX_RASTER_FONT_PX),
        "no rasterization past the ceiling"
    );
    let below = with_overrides(
        base.clone(),
        &[RunOverrideOp::Scale {
            range: 0..1,
            value: MAX_RASTER_FONT_PX / SIZE * 0.5,
        }],
    );
    assert_eq!(
        emit(&below, &camera).1.skipped,
        0,
        "control: a large scale under the ceiling is drawn"
    );

    let mut element = GfxElement::new_run(base, 0, 1);
    apply_mutations_to_element(&[GfxMutation::RunOpacity(f32::NAN)], &mut element);
    let nan_opacity = element.glyph_run().cloned().expect("a run");
    assert!(
        emit(&nan_opacity, &camera).0.is_empty(),
        "a NaN opacity draws nothing"
    );
}

/// **The canvas pass holds a run without shaping it, and hands it to the
/// glyph pass with its tree's offset.** A tree holding one run under its
/// (`Void`) root. The first pass holds the run (`recaptured`) and shapes
/// nothing for it — its one `reshaped` is the root, which yields no
/// buffer. Every later pass runs with this thread holding the font
/// guard, so a pass that took the lock would panic on the re-entrant
/// acquire: an untouched pass does nothing, a moved tree is patched (the
/// run's offset follows), and a run whose opacity changed is held again,
/// all without the lock.
#[test]
fn test_glyph_run_canvas_pass_holds_runs_without_the_font_lock() {
    use crate::application::renderer::{refresh, ScenePassKind};
    use baumhard::engine::tree::Placement;
    use baumhard::gfx_structs::scene::Scene;
    use baumhard::gfx_structs::tree::GfxTree;

    use super::super::scene_shape_cache::glyph_runs;

    let run = path_run(
        shaped("AB"),
        vec![at(0, 10.0, 10.0), at(1, 30.0, 10.0)],
        Vec2::ZERO,
    );
    let mut tree: GfxTree = GfxTree::new_non_indexed();
    let node = tree
        .insert_child(
            tree.root(),
            GfxElement::new_run(run.clone(), 0, 5),
            Placement::Last,
        )
        .expect("a child of the root");
    let mut scene = Scene::new();
    let id = scene.insert(tree, 0, Vec2::new(3.0, 4.0));
    let mut held = Vec::new();
    let pass = |scene: &Scene, held: &mut Vec<_>| {
        // Held across the pass: a lock taken inside it would panic.
        with_fonts(|_guard| refresh(scene, &[id], held, ScenePassKind::Canvas))
    };

    let cold = refresh(&scene, &[id], &mut held, ScenePassKind::Canvas);
    assert_eq!(
        (cold.walked, cold.reshaped, cold.patched, cold.recaptured),
        (2, 1, 0, 1),
        "the root and the run walked; the run held, not shaped"
    );
    let runs: Vec<_> = glyph_runs(&held).collect();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].0.same_state(&run), "the run held is the run in the tree");
    assert_eq!(runs[0].1, Vec2::new(3.0, 4.0), "with its tree's offset");

    let warm = pass(&scene, &mut held);
    assert_eq!(
        (warm.reshaped, warm.patched, warm.recaptured),
        (0, 0, 0),
        "nothing changed"
    );

    scene.set_offset(id, Vec2::new(8.0, 4.0));
    let moved = pass(&scene, &mut held);
    assert_eq!(
        (moved.patched, moved.recaptured),
        (2, 0),
        "a moved tree moves the root and the run, and holds nothing again"
    );
    assert_eq!(
        glyph_runs(&held).next().map(|(_, offset)| offset),
        Some(Vec2::new(8.0, 4.0))
    );

    scene
        .tree_mut(id)
        .expect("registered")
        .update(node, |element| {
            apply_mutations_to_element(&[GfxMutation::RunOpacity(0.25)], element)
        })
        .expect("the node exists");
    let faded = pass(&scene, &mut held);
    assert_eq!(
        (faded.reshaped, faded.recaptured),
        (0, 1),
        "a changed run is held again"
    );
    assert_eq!(glyph_runs(&held).next().map(|(run, _)| run.opacity()), Some(0.25));
}
