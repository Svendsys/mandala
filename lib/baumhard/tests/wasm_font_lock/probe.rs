// SPDX-License-Identifier: MPL-2.0

//! The wasm32 side of `tests/wasm_font_lock.rs`: a `cdylib` that
//! exports one function per scenario, each calling baumhard's real
//! font-lock helpers — or, in `flow_shape_and_place`, TextFlow's
//! shaping and placement under the guard, and in
//! `deep_mutator_validates`, the document validator's deep walk, and in
//! `deep_glyph_chain_hits`, the glyph tree's hit test over runs and
//! fills 100,000 levels down — so a node runner can execute them in a
//! real WebAssembly engine.
//!
//! This file is not part of any cargo target in the repository. The
//! test copies it into a staging crate under `target/` and builds it
//! there, against the workspace's own `Cargo.lock`, so it compiles the
//! dependency graph the app ships rather than a second one that could
//! drift.
//!
//! Every export returns `1` on success; a probe that checks several
//! things returns a distinct other number for each that went wrong.
//! The `reentrant_*` exports are *meant* to panic: on
//! `wasm32-unknown-unknown` a panic aborts as a trap, which says
//! nothing about its cause, so the hook below leaves the message where
//! the host can read it. The runner decides pass or
//! fail from that message, not from the trap: a trap on its own is
//! exactly what `Instant::now()` produces too, so only the message
//! tells a deliberate panic from a bug.
//!
//! **Plain safe Rust, by design.** Baumhard keeps its source free of
//! the keyword this file would otherwise need for a host callback
//! (`lib/baumhard/CONVENTIONS.md`), and a probe that lives under
//! `lib/baumhard/tests/` is held to the same rule. So there is no host
//! import to call. The hook writes the message into a static buffer,
//! the module exports the *addresses* of that buffer and its length,
//! and the host reads linear memory directly. It fetches the two
//! addresses before it runs a scenario, so it never has to call into
//! an instance that has already trapped.

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Once;

use baumhard::document::resolve::ResolvedText;
use baumhard::document::values::{Align, Direction, Edge, FitMode, FontSize, Percent, Rgba, Shape, StartPoint, Weight};
use baumhard::flow::{outline, place_on_closed, place_on_open, shape_box, shape_run, Curve, FlowSpan, Rect};
use baumhard::font::fonts::{
    self, acquire_font_system_read, acquire_font_system_write, try_acquire_font_system_write, AppFont,
};
use baumhard::glam::Vec2;

/// Longest panic message kept. Longer ones are truncated; every message
/// the runner looks for is in the first line.
const MESSAGE_CAP: usize = 1024;

/// The last panic's message. `AtomicU8` has the same in-memory
/// representation as `u8`, which is what lets the host read these as
/// plain bytes.
static MESSAGE: [AtomicU8; MESSAGE_CAP] = [const { AtomicU8::new(0) }; MESSAGE_CAP];
static MESSAGE_LEN: AtomicUsize = AtomicUsize::new(0);

fn record_panic(message: &str) {
    let bytes = message.as_bytes();
    let kept = bytes.len().min(MESSAGE_CAP);
    for (slot, byte) in MESSAGE.iter().zip(bytes) {
        slot.store(*byte, Ordering::Relaxed);
    }
    MESSAGE_LEN.store(kept, Ordering::Relaxed);
}

/// Where the host finds the panic message bytes in linear memory.
#[no_mangle]
pub extern "C" fn panic_message_addr() -> usize {
    MESSAGE.as_ptr() as usize
}

/// Where the host finds the message's length (a `usize`, 4 bytes here).
#[no_mangle]
pub extern "C" fn panic_message_len_addr() -> usize {
    std::ptr::from_ref(&MESSAGE_LEN) as usize
}

fn install_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| record_panic(&info.to_string())));
    });
}

/// Take the write guard, use it, release it, take it again. The second
/// acquire proves the release cleared the thread's holding record, and
/// the deref proves the guard type reaches the `FontSystem`.
#[no_mangle]
pub extern "C" fn uncontended_write() -> u32 {
    install_hook();
    let first = acquire_font_system_write("probe::first");
    let _faces = first.db().len();
    drop(first);
    let second = acquire_font_system_write("probe::second");
    drop(second);
    1
}

#[no_mangle]
pub extern "C" fn uncontended_try_write() -> u32 {
    install_hook();
    u32::from(try_acquire_font_system_write("probe::try").is_some())
}

/// Two nested reads on one thread are legal, and are the case a plain
/// (non-recursive) read could deadlock behind a queued writer.
#[no_mangle]
pub extern "C" fn uncontended_read() -> u32 {
    install_hook();
    let outer = acquire_font_system_read("probe::outer");
    let inner = acquire_font_system_read("probe::inner");
    let _faces = inner.db().len();
    drop(inner);
    drop(outer);
    1
}

#[no_mangle]
pub extern "C" fn reentrant_write_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = acquire_font_system_write("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_write_try_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = try_acquire_font_system_write("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_write_read() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = acquire_font_system_read("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_read_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_read("probe::outer");
    let _inner = acquire_font_system_write("probe::inner");
    0
}

/// TextFlow's shaping and placement, run for real: shape a run and a
/// box under the guard, then place the run around an outline with
/// `Fit` (which reshapes) and along a line with a `Once` filler (which
/// shapes the filler). This is the path every frame, connector and
/// section takes in the browser, and the one most likely to reach a
/// clock or a thread the browser has not got.
#[no_mangle]
pub extern "C" fn flow_shape_and_place() -> u32 {
    install_hook();
    fonts::init();
    let style = ResolvedText {
        font: AppFont::GalatiaSIL,
        size: FontSize::Pt(14.0),
        color: Rgba::opaque(255, 255, 255),
        weight: Weight::BOLD,
        italic: true,
    };
    let span = |text| FlowSpan {
        text,
        style: &style,
        link: None,
    };
    let mut fs = acquire_font_system_write("probe::flow");
    let run = shape_run(&[span("abc")], 1.0, &mut fs);
    if run.glyphs.len() != 3 {
        return 2;
    }
    let shaped = shape_box(&[span("ab\ncd")], None, Align::Center, &mut fs);
    if shaped.glyph_spans.len() != 4 || shaped.buffer.layout_runs().count() != 2 {
        return 3;
    }
    let path = outline(
        Rect::new(Vec2::ZERO, Vec2::new(120.0, 80.0)),
        Shape::Rectangle {
            radius: Percent::new(30),
        },
    );
    let start = StartPoint {
        edge: Edge::Top,
        at: 0.5,
    };
    let fit = FitMode::Fit { min: 0.5, max: 2.0 };
    let (rescaled, placed) = place_on_closed(&run, &path, start, Direction::Clockwise, &fit, true, &style, &mut fs);
    if rescaled.is_none() || placed.glyphs.len() != 3 {
        return 4;
    }
    let filler = FitMode::Once {
        filler: Some("-".to_owned()),
    };
    let line = Curve::Line(Vec2::ZERO, Vec2::new(300.0, 0.0));
    let (filled, placed) = place_on_open(&run, &line, &filler, 0.0, &style, &mut fs);
    if filled.is_none() || placed.glyphs.len() <= 3 {
        return 5;
    }
    1
}

/// Validate a document mutator tree 100,000 levels deep (design §20
/// rule 11, ruling R14.4 on #237): the validator's walk iterates, so it
/// fits wasm32's default stack in a debug build. Runs the native test's
/// body, `do_document_validate_walks_a_deep_mutator`, which panics on
/// any failure.
#[no_mangle]
pub extern "C" fn deep_mutator_validates() -> u32 {
    install_hook();
    baumhard::document::tests::mutation_validate_tests::do_document_validate_walks_a_deep_mutator();
    1
}

/// The engine's and the document's walks over a chain 100,000 levels
/// deep (design §20 rule 11, V1-A5 #282): each export runs one native
/// test body, which panics on any failure. In a debug build on wasm32's
/// default stack, a walk that recursed once per level would trap here.
macro_rules! deep_scenario {
    ($name:ident, $body:path) => {
        #[no_mangle]
        pub extern "C" fn $name() -> u32 {
            install_hook();
            $body();
            1
        }
    };
}

deep_scenario!(deep_engine_deep_chain_full_walk, baumhard::engine::tests::deep_tests::do_engine_deep_chain_full_walk);
deep_scenario!(deep_engine_deep_chain_repeat_while_to_the_leaf, baumhard::engine::tests::deep_tests::do_engine_deep_chain_repeat_while_to_the_leaf);
deep_scenario!(deep_engine_deep_chain_anchored_walks, baumhard::engine::tests::deep_tests::do_engine_deep_chain_anchored_walks);
deep_scenario!(deep_engine_deep_chain_journaled_edit_at_the_leaf, baumhard::engine::tests::deep_tests::do_engine_deep_chain_journaled_edit_at_the_leaf);
deep_scenario!(deep_engine_deep_chain_moves_whole, baumhard::engine::tests::deep_tests::do_engine_deep_chain_moves_whole);
deep_scenario!(deep_engine_deep_subtree_inserts_and_is_a_value, baumhard::engine::tests::deep_tests::do_engine_deep_subtree_inserts_and_is_a_value);
deep_scenario!(deep_engine_deep_walks_inherit_once_per_level, baumhard::engine::tests::deep_tests::do_engine_deep_walks_inherit_once_per_level);
deep_scenario!(deep_document_deep_chain_full_walk, baumhard::document::tests::deep_tests::do_document_deep_chain_full_walk);
deep_scenario!(deep_document_deep_chain_repeat_while_to_the_leaf, baumhard::document::tests::deep_tests::do_document_deep_chain_repeat_while_to_the_leaf);
deep_scenario!(deep_document_deep_chain_journaled_edit_at_the_leaf, baumhard::document::tests::deep_tests::do_document_deep_chain_journaled_edit_at_the_leaf);
deep_scenario!(deep_document_deep_chain_moves_whole, baumhard::document::tests::deep_tests::do_document_deep_chain_moves_whole);
deep_scenario!(deep_document_deep_subtree_validates_and_inserts, baumhard::document::tests::deep_tests::do_document_deep_subtree_validates_and_inserts);
deep_scenario!(deep_document_deep_chain_resolves_at_the_leaf, baumhard::document::tests::deep_tests::do_document_deep_chain_resolves_at_the_leaf);
deep_scenario!(deep_document_validate_walks_a_deep_insert, baumhard::document::tests::mutation_validate_tests::do_document_validate_walks_a_deep_insert);

/// Hit-test a glyph tree 100,000 levels deep that ends in a fill and a
/// run (design §20 rule 11): the box passes that read a run's and a
/// fill's bounds iterate, so they fit wasm32's default stack in a debug
/// build. Runs the native test's body,
/// `do_hit_test_reaches_a_run_and_a_fill_in_a_deep_chain`, which panics
/// on any failure.
#[no_mangle]
pub extern "C" fn deep_glyph_chain_hits() -> u32 {
    install_hook();
    baumhard::gfx_structs::tests::glyph_layer_tests::do_hit_test_reaches_a_run_and_a_fill_in_a_deep_chain();
    1
}
