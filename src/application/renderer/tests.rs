// SPDX-License-Identifier: MPL-2.0

use super::*;
use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::area_fields::OutlineStyle;

// ====================================================================
// Console overlay layout
// ====================================================================

fn empty_console_geometry() -> ConsoleOverlayGeometry {
    ConsoleOverlayGeometry {
        input: String::new(),
        cursor_grapheme: 0,
        scrollback: Vec::new(),
        completions: Vec::new(),
        selected_completion: None,
        default_font: None,
        font_size: 16.0,
    }
}

fn sample_console_geometry() -> ConsoleOverlayGeometry {
    ConsoleOverlayGeometry {
        input: "anchor set from t".to_string(),
        cursor_grapheme: 17,
        scrollback: vec![
            ConsoleOverlayLine {
                text: "> help".to_string(),
                kind: ConsoleOverlayLineKind::Input,
                font_family: None,
            },
            ConsoleOverlayLine {
                text: "commands:".to_string(),
                kind: ConsoleOverlayLineKind::Output,
                font_family: None,
            },
        ],
        completions: vec![ConsoleOverlayCompletion {
            text: "top".to_string(),
            hint: None,
            font_family: None,
        }],
        selected_completion: Some(0),
        default_font: None,
        font_size: 16.0,
    }
}

#[test]
fn test_console_backdrop_matches_border_bounds_exactly() {
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    let (bd_left, bd_top, bd_w, bd_h) = layout.backdrop_rect();
    assert_eq!(bd_left, layout.left);
    assert_eq!(bd_top, layout.top);
    assert_eq!(bd_w, layout.frame_width);
    assert_eq!(bd_h, layout.frame_height + layout.font_size);
}

#[test]
fn test_console_backdrop_has_no_horizontal_overhang() {
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    let (bd_left, _, bd_w, _) = layout.backdrop_rect();
    let bd_right = bd_left + bd_w;
    let border_right = layout.left + layout.frame_width;
    assert!(bd_right <= border_right + 0.001);
    assert!(bd_left >= layout.left - 0.001);
}

#[test]
fn test_console_frame_is_bottom_anchored() {
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    // Bottom border glyph row extends `font_size` below frame_height.
    // Its bottom edge should sit within `inner_padding` of the
    // screen bottom.
    let frame_bottom = layout.top + layout.frame_height + layout.font_size;
    let gap = 1080.0 - frame_bottom;
    assert!(
        gap <= layout.inner_padding + 0.5 && gap >= 0.0,
        "frame not bottom-anchored: gap={gap}"
    );
}

#[test]
fn test_console_frame_height_linear_in_scrollback_rows() {
    let g_empty = empty_console_geometry();
    let mut g_one = empty_console_geometry();
    g_one.scrollback.push(ConsoleOverlayLine {
        text: "one".into(),
        kind: ConsoleOverlayLineKind::Output,
        font_family: None,
    });
    let mut g_two = g_one.clone();
    g_two.scrollback.push(ConsoleOverlayLine {
        text: "two".into(),
        kind: ConsoleOverlayLineKind::Output,
        font_family: None,
    });
    let h0 = compute_console_frame_layout(&g_empty, 1920.0, 1080.0).frame_height;
    let h1 = compute_console_frame_layout(&g_one, 1920.0, 1080.0).frame_height;
    let h2 = compute_console_frame_layout(&g_two, 1920.0, 1080.0).frame_height;
    let delta1 = h1 - h0;
    let delta2 = h2 - h1;
    assert!((delta1 - delta2).abs() < 0.01);
}

#[test]
fn test_console_scrollback_clamped_to_max_rows() {
    let mut geometry = empty_console_geometry();
    for i in 0..100 {
        geometry.scrollback.push(ConsoleOverlayLine {
            text: format!("line {i}"),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        });
    }
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    assert_eq!(layout.scrollback_rows, MAX_CONSOLE_SCROLLBACK_ROWS);
}

#[test]
fn test_console_completions_clamped_to_max_rows() {
    let mut geometry = empty_console_geometry();
    for i in 0..100 {
        geometry.completions.push(ConsoleOverlayCompletion {
            text: format!("cmd_{i}"),
            hint: None,
            font_family: None,
        });
    }
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    assert_eq!(layout.completion_rows, MAX_CONSOLE_COMPLETION_ROWS);
}

#[test]
fn test_console_frame_is_full_window_width() {
    // The console is a bottom-anchored full-width strip with a
    // small horizontal margin on each side. Frame width + 2 ×
    // margin should sum to roughly the screen width.
    let layout = compute_console_frame_layout(&empty_console_geometry(), 1920.0, 1080.0);
    let total = layout.left * 2.0 + layout.frame_width;
    assert!((total - 1920.0).abs() < 1.0, "frame doesn't span full width");
}

#[test]
fn test_console_frame_width_independent_of_scrollback_len() {
    // With the full-width layout, a long scrollback line cannot
    // push the frame wider — it's clipped by the content area.
    let short = compute_console_frame_layout(&empty_console_geometry(), 1920.0, 1080.0).frame_width;
    let mut huge = empty_console_geometry();
    huge.scrollback.push(ConsoleOverlayLine {
        text: "x".repeat(500),
        kind: ConsoleOverlayLineKind::Output,
        font_family: None,
    });
    let long = compute_console_frame_layout(&huge, 1920.0, 1080.0).frame_width;
    assert_eq!(short, long);
}

#[test]
fn test_console_frame_width_stable_for_wide_char_scrollback() {
    // Backdrop-vs-border alignment with a wide-char line — the
    // content is truncated by baumhard's `truncate_to_display_width`
    // so it can't blow past the right border, and the frame
    // itself is still the full window width.
    let mut g = empty_console_geometry();
    g.scrollback.push(ConsoleOverlayLine {
        text: "日本語".repeat(200),
        kind: ConsoleOverlayLineKind::Output,
        font_family: None,
    });
    let layout = compute_console_frame_layout(&g, 1920.0, 1080.0);
    let (bd_left, _, bd_w, _) = layout.backdrop_rect();
    assert_eq!(bd_left, layout.left);
    assert_eq!(bd_w, layout.frame_width);
}

// -----------------------------------------------------------------
// Console border source-string tests
//
// The border draw uses baumhard's `BorderGlyphSet::box_drawing_rounded`
// via `build_console_border_strings(cols, rows)`.
// -----------------------------------------------------------------

#[test]
fn test_console_border_uses_rounded_corners() {
    let (top, bottom, _, _) = build_console_border_strings(10, 4);
    let top_chars: Vec<char> = top.chars().collect();
    let bot_chars: Vec<char> = bottom.chars().collect();
    assert_eq!(top_chars[0], '\u{256D}'); // ╭
    assert_eq!(*top_chars.last().unwrap(), '\u{256E}'); // ╮
    assert_eq!(bot_chars[0], '\u{2570}'); // ╰
    assert_eq!(*bot_chars.last().unwrap(), '\u{256F}'); // ╯
                                                        // Middle chars of the top border are `─`.
    for c in &top_chars[1..top_chars.len() - 1] {
        assert_eq!(*c, '\u{2500}');
    }
}

#[test]
fn test_console_border_top_row_length_matches_cols() {
    // `cols` = total border length including both corners.
    let (top, bottom, _, _) = build_console_border_strings(20, 4);
    assert_eq!(top.chars().count(), 20);
    assert_eq!(bottom.chars().count(), 20);
}

#[test]
fn test_console_border_sides_one_char_per_line() {
    let (_, _, left, right) = build_console_border_strings(10, 5);
    // One `│` per line, newline-separated; 5 lines total.
    assert_eq!(left.lines().count(), 5);
    assert_eq!(right.lines().count(), 5);
    for line in left.lines() {
        assert_eq!(line.chars().count(), 1);
        assert_eq!(line.chars().next().unwrap(), '\u{2502}');
    }
}

#[test]
fn test_console_border_scales_with_cols_and_rows() {
    let (top_narrow, _, left_short, _) = build_console_border_strings(10, 3);
    let (top_wide, _, left_tall, _) = build_console_border_strings(40, 10);
    assert!(top_wide.chars().count() > top_narrow.chars().count());
    assert!(left_tall.lines().count() > left_short.lines().count());
}

#[test]
fn test_console_prompt_y_sits_below_scrollback_and_completions() {
    // Regression guard for the overlap bug where `prompt_y`
    // floated at `frame_height - inner_padding - font_size`,
    // landing ~0.6 · font_size *above* the last completion row
    // instead of below it.
    let mut g = empty_console_geometry();
    g.scrollback = vec![
        ConsoleOverlayLine {
            text: "one".into(),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        },
        ConsoleOverlayLine {
            text: "two".into(),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        },
    ];
    g.completions = vec![ConsoleOverlayCompletion {
        text: "help".into(),
        hint: None,
        font_family: None,
    }];
    g.selected_completion = Some(0);
    let layout = compute_console_frame_layout(&g, 1920.0, 1080.0);

    let content_top = layout.top + layout.font_size + layout.inner_padding;
    let last_completion_end =
        content_top + layout.row_height * (layout.scrollback_rows + layout.completion_rows) as f32;
    assert!(
        layout.prompt_y() >= last_completion_end - 0.01,
        "prompt_y {} overlaps last completion row ending at {}",
        layout.prompt_y(),
        last_completion_end
    );
}

#[test]
fn test_console_prompt_y_fits_inside_frame() {
    // The prompt row plus its padded budget must stay inside
    // `frame_height`; otherwise it renders outside the border.
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    let prompt_bottom = layout.prompt_y() + layout.font_size * 1.4;
    let frame_bottom = layout.top + layout.frame_height;
    assert!(
        prompt_bottom <= frame_bottom + 0.01,
        "prompt bottom {} overruns frame bottom {}",
        prompt_bottom,
        frame_bottom
    );
}

#[test]
fn test_console_border_fills_full_frame_cols() {
    // The renderer picks `cols = floor(frame_width / char_width)`
    // and calls `build_console_border_strings(cols, rows)`, so
    // the top string always has exactly `cols` glyphs — one per
    // char-width cell.
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    let cols = (layout.frame_width / layout.char_width).floor() as usize;
    let (top, _, _, _) = build_console_border_strings(cols, 4);
    assert_eq!(top.chars().count(), cols);
}

#[test]
fn test_console_frame_layout_scales_with_font_size() {
    let mut g = empty_console_geometry();
    g.font_size = 8.0;
    let small = compute_console_frame_layout(&g, 1920.0, 1080.0);
    g.font_size = 32.0;
    let large = compute_console_frame_layout(&g, 1920.0, 1080.0);
    assert!(large.font_size > small.font_size);
    assert!(large.row_height > small.row_height);
    assert!(large.frame_height > small.frame_height);
}

/// Console round-trip: applying the mutator to a tree built
/// at state A leaves it byte-identical (per variable field) to
/// a fresh `build_console_overlay_tree(B)`. Pins the §B2
/// in-place update path for the keystroke hot path: the
/// dispatcher in `rebuild_console_overlay_buffers` takes this
/// branch on every input change frame.
#[test]
fn console_mutator_round_trips_to_fresh_build() {
    use baumhard::engine::element::TreeElement;
    baumhard::font::fonts::init();

    let mut g_a = sample_console_geometry();
    g_a.input = "anchor".into();
    g_a.cursor_grapheme = 6;
    let layout_a = compute_console_frame_layout(&g_a, 1280.0, 720.0);

    let mut g_b = sample_console_geometry();
    g_b.input = "anchor set".into();
    g_b.cursor_grapheme = 10;
    let layout_b = compute_console_frame_layout(&g_b, 1280.0, 720.0);

    // Same scrollback_rows / completion_rows means the
    // structural signature matches and the mutator is sound.
    assert_eq!(layout_a.scrollback_rows, layout_b.scrollback_rows);
    assert_eq!(layout_a.completion_rows, layout_b.completion_rows);

    let mut tree = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (overlay tree)");
        build_console_overlay_tree(&g_a, &layout_a, &mut fs)
    };
    let mutator = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (overlay mutator)");
        build_console_overlay_mutator(&g_b, &layout_b, &mut fs)
    };
    mutator.apply_to(&mut tree, &());

    // The tree keeps its children in channel order, and the areas
    // come out in channel order — every gutter, then every text —
    // so the two are compared as they stand.
    let expected = {
        let mut fs =
            baumhard::font::fonts::acquire_font_system_write("renderer::tests (overlay areas expected)");
        console_overlay_areas(&g_b, &layout_b, &mut fs)
    };

    let mut got: Vec<(usize, GlyphArea)> = Vec::new();
    for descendant_id in tree.root().descendants(tree.arena()) {
        let node = tree.arena().get(descendant_id).expect("arena node");
        let element = node.get();
        if let Some(area) = element.glyph_area() {
            got.push((element.channel(), area.clone()));
        }
    }

    assert_eq!(got.len(), expected.len(), "post-mutation element count");
    for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
        assert_eq!(c_got, c_exp, "channel mismatch");
        assert_eq!(a_got.text, a_exp.text, "text on ch {c_got}");
        assert_eq!(a_got.position, a_exp.position, "position on ch {c_got}");
        assert_eq!(a_got.regions, a_exp.regions, "regions on ch {c_got}");
    }

    // The signature itself must agree across the two layouts
    // (otherwise the dispatcher wouldn't take the mutator
    // branch in the first place).
    assert_eq!(
        console_overlay_signature(&layout_a),
        console_overlay_signature(&layout_b)
    );
}

/// Scrollback-grow shifts the structural signature — the
/// dispatcher must take the full-rebuild path, not the
/// in-place mutator path. Without this, a mutator computed
/// from N+1 scrollback entries applied to a tree built from
/// N would walk off the end and silently drop content. Pins
/// the structural-signature contract the dispatcher relies
/// on in `rebuild_console_overlay_buffers`.
#[test]
fn console_signature_shifts_on_scrollback_grow() {
    baumhard::font::fonts::init();

    let mut g_one = sample_console_geometry();
    g_one.scrollback = vec![ConsoleOverlayLine {
        text: "> help".into(),
        kind: ConsoleOverlayLineKind::Input,
        font_family: None,
    }];
    let layout_one = compute_console_frame_layout(&g_one, 1280.0, 720.0);

    let mut g_two = sample_console_geometry();
    g_two.scrollback = vec![
        ConsoleOverlayLine {
            text: "> help".into(),
            kind: ConsoleOverlayLineKind::Input,
            font_family: None,
        },
        ConsoleOverlayLine {
            text: "new output line".into(),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        },
    ];
    let layout_two = compute_console_frame_layout(&g_two, 1280.0, 720.0);

    assert_ne!(layout_one.scrollback_rows, layout_two.scrollback_rows);
    assert_ne!(
        console_overlay_signature(&layout_one),
        console_overlay_signature(&layout_two)
    );
}

/// `console_overlay_areas` degrades (logs + skips the slot) rather
/// than panicking when a caller violates the
/// `scrollback_rows = min(scrollback.len(), MAX)` (or
/// `completion_rows` mirror) invariant — interactive paths never
/// abort (§9). Pin the degraded behavior: artificially shorten the
/// geometry's scrollback vec AFTER computing the layout so
/// `scrollback_rows` (baked into the layout) exceeds
/// `geometry.scrollback.len()`, then call `console_overlay_areas`
/// and assert we return without panic.
///
/// A regression to `.expect()` would poison the test thread; the
/// surviving return proves the defensive path still fires.
#[test]
fn console_overlay_areas_degrades_when_scrollback_shorter_than_layout_rows() {
    baumhard::font::fonts::init();

    let mut g = sample_console_geometry();
    // Populate enough scrollback entries for the layout to reserve
    // several rows, then truncate AFTER layout so the
    // `scrollback_rows` count in the layout outruns the vec's length.
    g.scrollback = (0..5)
        .map(|i| ConsoleOverlayLine {
            text: format!("line {i}"),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        })
        .collect();
    g.completions = Vec::new();
    let layout = compute_console_frame_layout(&g, 1280.0, 720.0);
    assert!(layout.scrollback_rows >= 1, "layout must reserve rows");

    // Evict scrollback so layout.scrollback_rows > geometry.scrollback.len().
    g.scrollback.clear();

    let areas = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (scrollback degrade)");
        console_overlay_areas(&g, &layout, &mut fs)
    };
    // Survival check: we got here without aborting. Every slot the
    // degraded path skipped dropped out of the output, but the
    // prompt / border / empty-completion slots still emit.
    assert!(!areas.is_empty(), "non-scrollback slots still render");
}

/// Mirror guard for the completion-popup slot. Populate completions
/// enough for the layout to reserve rows, clear the vec AFTER
/// layout, then call `console_overlay_areas` and assert no panic.
#[test]
fn console_overlay_areas_degrades_when_completions_shorter_than_layout_rows() {
    baumhard::font::fonts::init();

    let mut g = sample_console_geometry();
    g.scrollback = Vec::new();
    g.completions = (0..3)
        .map(|i| ConsoleOverlayCompletion {
            text: format!("cand{i}"),
            hint: None,
            font_family: None,
        })
        .collect();
    g.selected_completion = Some(0);
    let layout = compute_console_frame_layout(&g, 1280.0, 720.0);
    assert!(layout.completion_rows >= 1, "layout must reserve rows");

    g.completions.clear();
    g.selected_completion = None;

    let areas = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (completion degrade)");
        console_overlay_areas(&g, &layout, &mut fs)
    };
    assert!(!areas.is_empty(), "non-completion slots still render");
}

/// Two distinct loaded faces, as `(family name, face)` pairs, for the
/// console default-face tests: one to configure as `console_font` and
/// one for a row to pin, so "the row kept its own face" and "the row
/// took the default" are different observations.
fn two_distinct_console_faces() -> ((String, AppFont), (String, AppFont)) {
    let mut faces = baumhard::font::fonts::loaded_families_iter().filter_map(|family| {
        baumhard::font::fonts::app_font_by_family(family).map(|f| (family.to_string(), f))
    });
    let first = faces.next().expect("the build ships at least one family");
    let second = faces
        .find(|(_, face)| *face != first.1)
        .expect("the build ships at least two distinct faces");
    (first, second)
}

/// Every region's font pin on the area at `channel`.
fn fonts_on_channel(areas: &[(usize, GlyphArea)], channel: usize) -> Vec<Option<AppFont>> {
    let (_, area) = areas
        .iter()
        .find(|(c, _)| *c == channel)
        .unwrap_or_else(|| panic!("no console area on channel {channel}"));
    area.regions.all_regions().iter().map(|r| r.font).collect()
}

/// A geometry exercising every way a console row can arrive: unpinned,
/// pinned to a loaded face, and pinned to a family that is not loaded;
/// plus a pinned completion with a hint and an unpinned one.
fn console_geometry_with_faces(default_font: Option<AppFont>, pinned_family: &str) -> ConsoleOverlayGeometry {
    let line = |text: &str, font_family: Option<&str>| ConsoleOverlayLine {
        text: text.to_string(),
        kind: ConsoleOverlayLineKind::Output,
        font_family: font_family.map(str::to_string),
    };
    ConsoleOverlayGeometry {
        input: "font set".to_string(),
        cursor_grapheme: 8,
        scrollback: vec![
            line("unpinned", None),
            line("pinned", Some(pinned_family)),
            line("pinned to nothing loaded", Some("NoSuchConsoleFace-7c2e")),
        ],
        completions: vec![
            ConsoleOverlayCompletion {
                text: pinned_family.to_string(),
                hint: Some("a family".to_string()),
                font_family: Some(pinned_family.to_string()),
            },
            ConsoleOverlayCompletion {
                text: "list".to_string(),
                hint: None,
                font_family: None,
            },
        ],
        selected_completion: Some(0),
        default_font,
        font_size: 16.0,
    }
}

/// `console_font` reaches every part of the console that does not pin
/// a face of its own — the frame, the gutters, the prompt, unpinned
/// rows, a completion's marker and hint, and a row whose pinned family
/// is not loaded — while a row pinned to a loaded face keeps it. This
/// is the consumer end of the setting: before #125 the overlay carried
/// the family and no area read it.
#[test]
fn test_console_default_font_reaches_every_unpinned_area() {
    baumhard::font::fonts::init();
    let ((_, console_face), (pinned_family, pinned_face)) = two_distinct_console_faces();

    let areas_for = |default_font: Option<AppFont>| {
        let g = console_geometry_with_faces(default_font, &pinned_family);
        let layout = compute_console_frame_layout(&g, 1280.0, 720.0);
        assert_eq!((layout.scrollback_rows, layout.completion_rows), (3, 2));
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (console faces)");
        console_overlay_areas(&g, &layout, &mut fs)
    };

    let configured = areas_for(Some(console_face));
    let d = Some(console_face);
    for channel in [
        CONSOLE_CHANNEL_TOP_BORDER,
        CONSOLE_CHANNEL_BOTTOM_BORDER,
        CONSOLE_CHANNEL_LEFT_COL,
        CONSOLE_CHANNEL_RIGHT_COL,
        CONSOLE_CHANNEL_SCROLLBACK_GUTTER_BASE + 1,
        CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE,
        CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE + 2,
        CONSOLE_CHANNEL_COMPLETION_BASE + 1,
    ] {
        let fonts = fonts_on_channel(&configured, channel);
        assert!(
            !fonts.is_empty() && fonts.iter().all(|f| *f == d),
            "channel {channel} must shape in console_font: {fonts:?}"
        );
    }
    assert_eq!(fonts_on_channel(&configured, CONSOLE_CHANNEL_PROMPT), vec![d, d]);
    assert_eq!(
        fonts_on_channel(&configured, CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE + 1),
        vec![Some(pinned_face)],
        "a row pinned to a loaded face keeps it"
    );
    assert_eq!(
        fonts_on_channel(&configured, CONSOLE_CHANNEL_COMPLETION_BASE),
        vec![d, Some(pinned_face), d],
        "a pinned candidate shapes its text in its face and its marker and hint in console_font"
    );

    // Control: with no console_font, the same unpinned areas carry no
    // pin at all, so every `d` above came from the configured face —
    // and the pinned row is unchanged, so the pin is not the default's
    // doing either.
    let unconfigured = areas_for(None);
    for channel in [
        CONSOLE_CHANNEL_TOP_BORDER,
        CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE,
        CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE + 2,
        CONSOLE_CHANNEL_PROMPT,
    ] {
        let fonts = fonts_on_channel(&unconfigured, channel);
        assert!(
            fonts.iter().all(Option::is_none),
            "channel {channel} has no console_font to take: {fonts:?}"
        );
    }
    assert_eq!(
        fonts_on_channel(&unconfigured, CONSOLE_CHANNEL_SCROLLBACK_TEXT_BASE + 1),
        vec![Some(pinned_face)]
    );
}

/// A completion row splits into marker / candidate / hint, each clamped
/// to what survived truncation, and never yields an empty or
/// out-of-bounds region.
#[test]
fn test_completion_row_regions_split_and_clamp_to_the_shown_width() {
    baumhard::font::fonts::init();
    let ((_, d), (_, c)) = two_distinct_console_faces();
    let color = [1.0, 1.0, 1.0, 1.0];
    let spans_of = |marker, text, shown| {
        completion_row_regions(
            CompletionRowSpans { marker, text, shown },
            color,
            Some(d),
            Some(c),
        )
        .all_regions()
        .iter()
        .map(|r| (r.range.start, r.range.end, r.font))
        .collect::<Vec<_>>()
    };

    // `▸ ` + `Norse` + `    a family`, all shown.
    assert_eq!(
        spans_of(2, 5, 19),
        vec![(0, 2, Some(d)), (2, 7, Some(c)), (7, 19, Some(d))]
    );
    // No hint: nothing after the candidate, so no trailing region.
    assert_eq!(spans_of(2, 5, 7), vec![(0, 2, Some(d)), (2, 7, Some(c))]);
    // Truncated inside the candidate: its region stops at the edge.
    assert_eq!(spans_of(2, 5, 4), vec![(0, 2, Some(d)), (2, 4, Some(c))]);
    // Truncated inside the marker: only the marker's shown part.
    assert_eq!(spans_of(2, 5, 1), vec![(0, 1, Some(d))]);
    // Nothing shown: no regions at all.
    assert_eq!(spans_of(2, 5, 0), vec![]);
}

/// A shipped face that tiles the console's border at a different
/// pitch from cosmic-text's default, with that pitch. The pitch is the
/// wider of the two rules the border draws — `─` and `│`, as
/// `console_overlay_areas` measures it — computed here from the
/// per-glyph advances rather than through `measure_max_glyph_advance`,
/// so the expectation does not quote the subject. Without such a face,
/// "measured in the configured face" and "measured in the default
/// face" are the same number and nothing below can tell them apart.
fn face_with_distinct_rule_advance(fs: &mut baumhard::font::FontSystem, size: f32) -> (AppFont, f32) {
    use baumhard::font::metric_cache::glyph_advance_with;
    let mut pitch = |face: Option<AppFont>| {
        glyph_advance_with(fs, face, size, "\u{2500}").max(glyph_advance_with(fs, face, size, "\u{2502}"))
    };
    let default_w = pitch(None);
    baumhard::font::fonts::loaded_families_iter()
        .filter_map(baumhard::font::fonts::app_font_by_family)
        .find_map(|face| {
            let w = pitch(Some(face));
            (w > 0.0 && w != default_w).then_some((face, w))
        })
        .expect("some shipped face tiles the border rules at a different pitch than the default")
}

/// `measure_max_glyph_advance` measures in the face it is given. A
/// face whose box-drawing rules have a different advance from the
/// default must measure differently, or a configured `console_font`
/// tiles its border at the wrong pitch.
#[test]
fn test_measure_max_glyph_advance_measures_in_the_given_face() {
    baumhard::font::fonts::init();
    let size = 18.0;
    let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (face advance)");
    let (face, face_w) = face_with_distinct_rule_advance(&mut fs, size);
    assert_eq!(
        measure_max_glyph_advance(&mut fs, Some(face), &["\u{2500}", "\u{2502}"], size),
        face_w
    );
    // Control: the same call without the face answers the default's
    // width, so the line above is the face pin's doing.
    assert_ne!(
        measure_max_glyph_advance(&mut fs, None, &["\u{2500}", "\u{2502}"], size),
        face_w
    );
}

/// The console frame tiles its right border at the column pitch of the
/// face the border draws in — `console_font` when configured. Measured
/// in the default face instead, the right column lands where the
/// default face's columns would end, off the configured face's grid.
#[test]
fn test_console_frame_pitch_is_measured_in_console_font() {
    baumhard::font::fonts::init();
    let size = 16.0;
    let (face, face_w) = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (pitch face)");
        face_with_distinct_rule_advance(&mut fs, size)
    };
    let right_col_x = |default_font: Option<AppFont>| {
        let g = ConsoleOverlayGeometry {
            default_font,
            font_size: size,
            ..sample_console_geometry()
        };
        let layout = compute_console_frame_layout(&g, 1280.0, 720.0);
        let areas = {
            let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (pitch areas)");
            console_overlay_areas(&g, &layout, &mut fs)
        };
        let (_, right) = areas
            .iter()
            .find(|(c, _)| *c == CONSOLE_CHANNEL_RIGHT_COL)
            .expect("the right border always emits");
        (layout, right.position.x())
    };

    let (layout, configured_x) = right_col_x(Some(face));
    let cols = ((layout.frame_width / face_w).floor() as usize).max(2);
    assert!(
        baumhard::util::geometry::almost_equal(configured_x, layout.left + (cols - 1) as f32 * face_w),
        "right border at {configured_x}, expected column {} of a {face_w}px grid from {}",
        cols - 1,
        layout.left
    );
    // Control: unconfigured, the border sits elsewhere — the grid above
    // is the configured face's, not one every console uses.
    let (_, unconfigured_x) = right_col_x(None);
    assert!(
        !baumhard::util::geometry::almost_equal(configured_x, unconfigured_x),
        "both at {configured_x}: the pitch did not follow console_font"
    );
}

/// Freeze-hardening regression: the surface-size clamp must leave
/// dimensions untouched when both axes are within the GPU's
/// `max_texture_dimension_2d` budget. Picking up an oversize
/// request silently (not clamping at all) would defeat the guard;
/// clamping when we didn't need to would spuriously letterbox.
#[test]
fn clamp_surface_size_is_identity_below_limit() {
    // A typical 4K panel in landscape — well under any modern
    // GPU's 2D texture limit (typically 8192 or 16384).
    let (w, h) = clamp_surface_size_to_gpu_limit(3840, 2160, 8192);
    assert_eq!((w, h), (3840, 2160));
}

/// The clamp must pin each axis that exceeds the GPU limit and
/// leave the other axis alone. Ultrawide-at-max on a modest GPU
/// is the realistic freeze-triggering scenario.
#[test]
fn clamp_surface_size_caps_only_the_oversized_axis() {
    // Width over, height fine.
    assert_eq!(clamp_surface_size_to_gpu_limit(10_000, 4096, 8192), (8192, 4096));
    // Height over, width fine.
    assert_eq!(clamp_surface_size_to_gpu_limit(4096, 10_000, 8192), (4096, 8192));
    // Both over — both pinned.
    assert_eq!(
        clamp_surface_size_to_gpu_limit(10_000, 12_000, 8192),
        (8192, 8192)
    );
}

/// Boundary: exactly at the limit is not clamped. The wgpu
/// contract is that dimensions **up to and including**
/// `max_texture_dimension_2d` are valid.
#[test]
fn clamp_surface_size_passes_exact_limit() {
    let (w, h) = clamp_surface_size_to_gpu_limit(8192, 8192, 8192);
    assert_eq!((w, h), (8192, 8192));
}

/// The map's background is the frame's clear value, written so the
/// frame holds the authored byte (V1-B12): on an sRGB target each byte
/// is decoded to linear light — the GPU encodes it back as it writes —
/// and on a linear target it goes in as it is. The expectations are
/// the sRGB transfer function's own values at three bytes (IEC
/// 61966-2-1: `0x80` is 0.2158605…, `0x0a` 0.0030352…), not a second
/// run of the code under test. Controls: the two targets disagree on
/// every byte but the ends, so the decode is not skipped on either;
/// and the alpha is 1 whatever the background's, since nothing is
/// behind the canvas.
#[test]
fn test_the_clear_color_holds_the_background_on_either_target() {
    use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba8UnormSrgb};
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    for srgb in [Bgra8UnormSrgb, Rgba8UnormSrgb] {
        let clear = clear_color_for([0x80, 0x0a, 0xff, 0x40], srgb);
        assert!(
            close(clear.r, 0.215_860_5),
            "{srgb:?}: 0x80 decodes, got {}",
            clear.r
        );
        assert!(
            close(clear.g, 0.003_035_27),
            "{srgb:?}: 0x0a decodes, got {}",
            clear.g
        );
        assert!(close(clear.b, 1.0), "{srgb:?}: 0xff is 1");
        assert_eq!(clear.a, 1.0, "{srgb:?}: opaque, whatever the alpha");
        let black = clear_color_for([0, 0, 0, 0], srgb);
        assert_eq!((black.r, black.g, black.b, black.a), (0.0, 0.0, 0.0, 1.0));
    }
    for linear in [Bgra8Unorm, Rgba8Unorm] {
        let clear = clear_color_for([0x80, 0x0a, 0xff, 0x40], linear);
        assert!(close(clear.r, 128.0 / 255.0), "{linear:?}: the byte as it is");
        assert!(close(clear.g, 10.0 / 255.0));
        assert!(close(clear.b, 1.0));
        assert_eq!(clear.a, 1.0);
    }
    for byte in 1..255u8 {
        let srgb = clear_color_for([byte, byte, byte, 255], Bgra8UnormSrgb).r;
        let linear = clear_color_for([byte, byte, byte, 255], Bgra8Unorm).r;
        assert!(srgb < linear, "0x{byte:02x}: an sRGB target's value is decoded");
    }
}

/// **A glyph colored like the background matches it exactly, on every
/// target** (CODE_CONVENTIONS §4, D12). For each surface format a
/// bootstrap can pick — native's and the headless renderer's
/// `Bgra8UnormSrgb`, the WebGL2 surface's `Rgba8UnormSrgb`, and the
/// WebGPU surface's `Bgra8Unorm` / `Rgba8Unorm` / `Rgba16Float`, none of
/// them sRGB — and for every authored byte, the byte each pass leaves in
/// the target is the authored byte:
///
/// - **the clear**, `clear_color_for`;
/// - **glyphon's text**, through the color mode the renderer gives its
///   atlas, the shader's per-mode handling of a glyph's color written
///   out here from glyphon 0.12.0's `shader.wgsl` (`ConvertToLinear`
///   applies the shader's sRGB-to-linear decode, `None` divides by 255);
/// - **the glyph-run pass's mask glyphs**, the real `instance`;
/// - **its color glyphs** (emoji), sampled from a page of the format the
///   pass gives it: an sRGB page decodes the stored byte, a linear one
///   returns it as it is.
///
/// "Leaves in the target" is the target's own write, written out from
/// its definition: an sRGB format encodes by the sRGB transfer function
/// (IEC 61966-2-1) and rounds; a linear one rounds the value as it is.
/// Controls: a pass that decodes on a linear target, or does not on an
/// sRGB one, leaves a darker or brighter byte for every byte but the
/// two ends — `0x80` lands as `0x37` or `0xbc` — which the last loop
/// asserts of the two wrong choices, so the agreement is not a model
/// that cannot tell them apart.
#[test]
fn test_the_three_color_passes_agree_on_every_target() {
    use super::glyph_runs::atlas::{Page, Slot};
    use super::glyph_runs::emit::GlyphSprite;
    use super::glyph_runs::instance;
    use super::target_encoding::TargetEncoding;
    use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba16Float, Rgba8Unorm, Rgba8UnormSrgb};

    let decode = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let encode = |l: f64| {
        if l <= 0.003_130_8 {
            l * 12.92
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        }
    };
    // The byte a target of `format` keeps for a pass's output `value`.
    let stored = |format: wgpu::TextureFormat, value: f64| -> u8 {
        let written = if format.is_srgb() { encode(value) } else { value };
        (written.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    // glyphon's shader: a glyph's color byte as the vertex stage hands
    // it on, per the conversion its color mode tags the glyph with.
    let glyphon = |mode: glyphon::ColorMode, byte: u8| -> f64 {
        let c = f32::from(byte) / 255.0;
        f64::from(match mode {
            glyphon::ColorMode::Accurate => {
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            }
            glyphon::ColorMode::Web => c,
        })
    };
    // A texture sample of a stored byte from a page of `format`.
    let sample = |format: wgpu::TextureFormat, byte: u8| -> f64 {
        let c = f64::from(byte) / 255.0;
        if format.is_srgb() {
            decode(c)
        } else {
            c
        }
    };
    let (key, _, _) = glyphon::CacheKey::new(
        glyphon::fontdb::ID::dummy(),
        0,
        16.0,
        (0.0, 0.0),
        glyphon::Weight::NORMAL,
        glyphon::cosmic_text::CacheKeyFlags::empty(),
    );
    let mask = Slot {
        page: Page::Mask,
        uv: (0, 0),
        size: (1, 1),
        left: 0,
        top: 0,
    };

    for format in [
        Bgra8UnormSrgb,
        Rgba8UnormSrgb,
        Bgra8Unorm,
        Rgba8Unorm,
        Rgba16Float,
    ] {
        let encoding = TargetEncoding::of(format);
        let mode = encoding.glyphon_color_mode();
        let page = encoding.color_page_format();
        for byte in 0..=255u8 {
            let clear = clear_color_for([byte, byte, byte, 255], format);
            let sprite = GlyphSprite {
                key,
                pen: (0, 0),
                color: [byte, byte, byte, 255],
                opacity: 1.0,
            };
            let run_glyph = instance(&sprite, mask, encoding).color;
            let left = [
                ("the clear", stored(format, clear.r)),
                ("glyphon's text", stored(format, glyphon(mode, byte))),
                ("a run's mask glyph", stored(format, f64::from(run_glyph[0]))),
                ("a run's color glyph", stored(format, sample(page, byte))),
            ];
            for (pass, got) in left {
                assert_eq!(
                    got, byte,
                    "{format:?}: {pass} leaves 0x{got:02x} for the authored 0x{byte:02x}"
                );
            }
        }
    }

    // The controls: on each kind of target, the other kind's choice
    // leaves a different byte, for every byte but the two ends.
    assert_eq!(
        stored(Bgra8Unorm, decode(128.0 / 255.0)),
        0x37,
        "a decode on a linear target darkens"
    );
    assert_eq!(
        stored(Bgra8UnormSrgb, 128.0 / 255.0),
        0xbc,
        "no decode on an sRGB target brightens"
    );
    for byte in 1..255u8 {
        let c = f64::from(byte) / 255.0;
        assert_ne!(
            stored(Bgra8Unorm, decode(c)),
            byte,
            "0x{byte:02x}: decoded on a linear target"
        );
        assert_ne!(
            stored(Bgra8UnormSrgb, c),
            byte,
            "0x{byte:02x}: not decoded on an sRGB target"
        );
    }
}

// --- FrameIntervalRing --------------------------------------------------
// Fundamentals coverage for the sum invariant backing
// `FpsDisplayMode::Debug`'s rolling average. Pure arithmetic, no clock.

#[test]
fn frame_interval_ring_new_is_empty() {
    let ring = FrameIntervalRing::new();
    assert_eq!(ring.avg_micros(), None, "empty ring has no average");
}

#[test]
fn frame_interval_ring_single_push_is_that_value() {
    let mut ring = FrameIntervalRing::new();
    ring.push(16_666);
    assert_eq!(ring.avg_micros(), Some(16_666));
}

#[test]
fn frame_interval_ring_partial_fill_averages_visible_samples() {
    let mut ring = FrameIntervalRing::new();
    ring.push(10);
    ring.push(20);
    ring.push(30);
    // Divisor is `filled` (3), not `FPS_WINDOW` — zero-padding the array
    // on cold start must not pull the reported average toward zero.
    assert_eq!(ring.avg_micros(), Some(20));
}

#[test]
fn frame_interval_ring_exact_fill_reports_uniform_value() {
    let mut ring = FrameIntervalRing::new();
    for _ in 0..FPS_WINDOW {
        ring.push(1_000);
    }
    assert_eq!(ring.avg_micros(), Some(1_000));
}

#[test]
fn frame_interval_ring_wrap_drops_oldest_sample() {
    let mut ring = FrameIntervalRing::new();
    // Seed with a distinctive sentinel so we can confirm it leaves the
    // window on wraparound.
    let sentinel = 999_999u128;
    ring.push(sentinel);
    for _ in 0..(FPS_WINDOW - 1) {
        ring.push(1_000);
    }
    // Ring is exactly full; sentinel + (FPS_WINDOW - 1) * 1000 in the
    // window. Average:
    //   (999_999 + 199 * 1000) / 200 = (999_999 + 199_000) / 200 = 5994
    let expected_with_sentinel = (sentinel + 1_000u128 * (FPS_WINDOW as u128 - 1)) / FPS_WINDOW as u128;
    assert_eq!(ring.avg_micros(), Some(expected_with_sentinel));

    // Push one more — the sentinel falls out of the window, and the
    // running sum must update accordingly. After this, the ring holds
    // FPS_WINDOW copies of 1_000.
    ring.push(1_000);
    assert_eq!(
        ring.avg_micros(),
        Some(1_000),
        "oldest sample must drop out of the rolling sum on wraparound"
    );
}

#[test]
fn frame_interval_ring_zero_value_still_occupies_slot() {
    let mut ring = FrameIntervalRing::new();
    ring.push(0);
    ring.push(200);
    // Two samples, sum 200 → avg 100. The zero push did NOT refuse the
    // slot; it contributed zero to the sum but advanced `filled`.
    assert_eq!(ring.avg_micros(), Some(100));
}

#[test]
fn frame_interval_ring_clear_restores_empty_state() {
    let mut ring = FrameIntervalRing::new();
    for i in 0..50 {
        ring.push((i + 1) as u128 * 100);
    }
    assert!(ring.avg_micros().is_some());
    ring.clear();
    assert_eq!(ring.avg_micros(), None);
    // And a fresh push lands cleanly on top — prior state did not
    // leak through clear().
    ring.push(42);
    assert_eq!(ring.avg_micros(), Some(42));
}

// ====================================================================
// Halo stamp bookkeeping
// ====================================================================

/// The walker stamps an outlined area's eight halos and then its glyph,
/// each at the area's anchor plus its stamp offset plus the tree offset:
/// halos first, so the glyph draws on top of them.
#[test]
fn test_the_walker_stamps_every_halo_then_the_glyph() {
    baumhard::font::fonts::init();

    let anchor = Vec2::new(11.0, 23.0);
    let tree_offset = Vec2::new(100.0, -50.0);
    let outline = OutlineStyle {
        color: [255, 0, 0, 255],
        px: 3.0,
    };
    let mut area = GlyphArea::new_with_str("halo", 16.0, 18.0, anchor, Vec2::new(80.0, 24.0));
    area.outline = Some(outline);
    let element = GfxElement::new_area_non_indexed_with_id(area, 1, 1);

    let mut emitted: Vec<(f32, f32)> = Vec::new();
    {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (halo stamps)");
        super::tree_walker::shape_one_element_into_buffers(
            &element,
            tree_offset,
            &mut fs,
            &mut |_uid, buffer| emitted.push(buffer.pos),
        );
    }

    let expected: Vec<(f32, f32)> = outline
        .offsets()
        .chain(std::iter::once((0.0, 0.0)))
        .map(|(dx, dy)| (anchor.x + dx + tree_offset.x, anchor.y + dy + tree_offset.y))
        .collect();
    assert_eq!(expected.len(), 9, "the halo technique is 8 stamps plus the glyph");
    assert_eq!(
        emitted, expected,
        "the halos at their offsets, then the glyph at the anchor, all moved by the tree offset"
    );
}

// ====================================================================
// Glyph advance measurement
// ====================================================================

/// Shape one cluster through a scratch `cosmic_text::Buffer` and
/// report `(widest layout glyph, summed layout glyphs, glyph count)`.
///
/// Deliberately *not* routed through
/// [`baumhard::font::metric_cache::glyph_advance`]: that is the path
/// `measure_max_glyph_advance` now takes, and an expectation
/// computed from the code under test cannot see a change in that
/// code. This is the scratch-buffer shaping the subject used before
/// the metric cache, kept here as an independent second opinion —
/// the first element of the tuple is precisely the answer the old
/// implementation gave.
fn shape_cluster_advances(
    font_system: &mut baumhard::font::FontSystem,
    cluster: &str,
    font_size: f32,
) -> (f32, f32, usize) {
    use baumhard::font::{buffer, SHAPING_ADVANCED};

    let mut buf = buffer::create_square(font_system, font_size);
    buf.set_text(cluster, &Attrs::new(), SHAPING_ADVANCED, None);
    buf.shape_until_scroll(font_system, false);
    let (mut widest, mut summed, mut count) = (0.0f32, 0.0f32, 0usize);
    for run in buf.layout_runs() {
        for glyph in run.glyphs.iter() {
            widest = widest.max(glyph.w);
            summed += glyph.w;
            count += 1;
        }
    }
    (widest, summed, count)
}

/// `measure_max_glyph_advance` answers with the widest of the
/// clusters it was given, and falls back to the monospace
/// approximation only when nothing shaped at all.
///
/// The measurement runs through baumhard's `(face, size, cluster)`
/// metric cache rather than a scratch `Buffer` per call, and that
/// move carried a semantic correction with it: a cluster that lays
/// out as several glyphs is now worth the *sum* of their advances,
/// where the scratch-buffer version took the widest one inside the
/// cluster and so under-measured it. The third assertion below is
/// the one that can see that: its cluster is a Devanagari
/// consonant-plus-vowel-sign that lays out as two glyphs, so sum and
/// max are different numbers and only one of them can pass.
///
/// Every expectation here comes from [`shape_cluster_advances`],
/// which shapes independently rather than asking the metric cache —
/// otherwise the test would be quoting the subject back at itself.
#[test]
fn test_measure_max_glyph_advance_takes_the_widest_and_falls_back_on_nothing() {
    use baumhard::font::metrics::monospace_advance;

    baumhard::font::fonts::init();
    let size = 18.0;

    let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (advance)");

    // A light horizontal rule and a full-block, which are not the
    // same width in any face this ships with. Both single-glyph, so
    // sum and max agree and this pair pins only the "max across the
    // set" half.
    let (narrow, wide) = ("\u{2500}", "\u{2588}");
    let (_, narrow_w, narrow_n) = shape_cluster_advances(&mut fs, narrow, size);
    let (_, wide_w, wide_n) = shape_cluster_advances(&mut fs, wide, size);
    assert!(narrow_w > 0.0 && wide_w > 0.0, "both clusters must shape");
    assert_eq!((narrow_n, wide_n), (1, 1), "both are single-glyph clusters");
    assert!(
        narrow_w < wide_w,
        "the two must differ, or 'widest' proves nothing"
    );
    assert_eq!(
        measure_max_glyph_advance(&mut fs, None, &[narrow, wide], size),
        wide_w,
        "the answer is the widest cluster in the set"
    );

    // Devanagari NA + vowel sign I: two layout glyphs, so the
    // cluster's summed advance and its widest single glyph are
    // different numbers. The scratch-buffer implementation answered
    // with the max and under-measured the cluster by the rest of it.
    let two_glyph = "\u{0928}\u{093F}";
    let (widest_in_cluster, summed, count) = shape_cluster_advances(&mut fs, two_glyph, size);
    assert!(
        count > 1 && summed > widest_in_cluster,
        "this cluster has to lay out as more than one glyph, or the routing below is untested: \
         {count} glyph(s), sum {summed}, widest {widest_in_cluster}"
    );
    assert_eq!(
        measure_max_glyph_advance(&mut fs, None, &[two_glyph], size),
        summed,
        "a multi-glyph cluster is worth the width it occupies, not the width of its widest piece"
    );

    // An empty cluster lays out no glyphs at all, so the max is
    // zero and the monospace approximation takes over — the branch
    // that keeps a tofu-only glyph set from collapsing the console
    // frame to zero columns.
    assert_eq!(
        measure_max_glyph_advance(&mut fs, None, &[""], size),
        monospace_advance(size),
        "a set that shapes to nothing must fall back rather than return zero"
    );
}

// ====================================================================
// Selection-highlight single-sourcing
// ====================================================================

/// Every reading of the selection cyan carries one RGB triple. The
/// three items compared here are declared in three places — the
/// view's tint (`view::overlay::SELECTION_HIGHLIGHT`) and the
/// renderer's two cosmic-text constants — and the assertion reads
/// each of them as a value rather than recomputing any of them, so it
/// is the re-inlining of a literal that turns it red.
///
/// That is exactly the input that made it red before this test
/// existed: the tree carried `#00E5FF` (green 229),
/// `[0.0, 0.9, 1.0, 1.0]` (229.5) and `Color::rgba(0, 230, 255, ..)`
/// twice (230) — three byte values for one affordance, each
/// individually plausible and none of them reachable from the
/// others.
///
/// Alpha is compared separately and only for the two opaque forms:
/// the rubber band's reduced alpha is the one thing about its color
/// that is genuinely its own.
#[test]
fn test_every_selection_highlight_form_carries_one_rgb_triple() {
    let tint = crate::application::view::overlay::SELECTION_HIGHLIGHT.0;
    let status = super::MODE_STATUS_COLOR;
    assert_eq!(
        [status.r(), status.g(), status.b(), status.a()],
        tint,
        "the mode-status line's cyan, opaque"
    );
    assert_eq!(tint[3], 255, "the selection tint is opaque");
}

// ====================================================================
// Backdrops as glyphs
// ====================================================================

/// **A backdrop cell is one opaque square at its text area's corner**,
/// which is what lets a grid of them tile a rectangle with no seam
/// (`backdrop`'s module docs). The cell is shaped as the renderer shapes
/// it, rasterized as glyphon rasterizes it (`SwashCache::get_image_uncached`
/// on the glyph's physical cache key, at scale 1 and a whole-pixel
/// corner), and placed by glyphon 0.12.0's own arithmetic from
/// `text_render.rs`: `x = physical.x + placement.left`, `y =
/// round(line_y) + physical.y − placement.top`. It must land at `(0, 0)`,
/// `CELL_PX` square, a coverage mask with every texel full — so a cell
/// at `(x, y)` covers exactly `x..x + 64` by `y..y + 64`, wholly.
///
/// Controls: the same block shaped half a pixel larger lands with
/// partial edge texels, and the default face's `█` is not a full square
/// — so the assertions tell a seamless cell from one that is not.
#[test]
fn test_a_backdrop_cell_is_one_opaque_square_at_its_corner() {
    use super::backdrop::{BackdropCell, CELL_PX};
    use glyphon::{SwashCache, SwashContent};

    baumhard::font::fonts::init();
    let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (backdrop cell)");
    // What glyphon draws of `buffer`'s one glyph at a text area whose
    // corner is the origin: where its ink lands, its size, its kind, and
    // how many of its texels are not fully covered.
    let mut swash = SwashCache::new();
    let mut ink = |buffer: &Buffer, fs: &mut baumhard::font::FontSystem| {
        let run = buffer.layout_runs().next().expect("the cell lays out a line");
        assert_eq!(run.glyphs.len(), 1, "the cell is one glyph");
        let physical = run.glyphs[0].physical((0.0, 0.0), 1.0);
        let image = swash
            .get_image_uncached(fs, physical.cache_key)
            .expect("the block rasterizes");
        let at = (
            physical.x + image.placement.left,
            run.line_y.round() as i32 + physical.y - image.placement.top,
        );
        let size = (image.placement.width, image.placement.height);
        let partial = image.data.iter().filter(|&&texel| texel != u8::MAX).count();
        (at, size, image.content, partial)
    };

    let cell = BackdropCell::shape(&mut fs);
    let (at, size, content, partial) = ink(cell.buffer(), &mut fs);
    assert_eq!(at, (0, 0), "the block's ink starts at the text area's corner");
    assert_eq!(
        size,
        (CELL_PX as u32, CELL_PX as u32),
        "the block's ink is the cell"
    );
    assert!(
        matches!(content, SwashContent::Mask),
        "the block is a coverage mask, tinted black"
    );
    assert_eq!(partial, 0, "every texel of the block is fully covered");

    // The controls: a half-pixel-larger block, and the default face's.
    let larger = super::backdrop::shape_block(&mut fs, CELL_PX as f32 + 0.5, Some(AppFont::PublicPixel));
    let (_, _, _, partial) = ink(&larger, &mut fs);
    assert!(
        partial > 0,
        "the control: a block off the pixel grid has partly covered edges"
    );
    let default_face = super::backdrop::shape_block(&mut fs, CELL_PX as f32, None);
    let (at, size, _, partial) = ink(&default_face, &mut fs);
    assert!(
        at != (0, 0) || size != (CELL_PX as u32, CELL_PX as u32) || partial > 0,
        "the control: the default face's block is not a full cell ({at:?}, {size:?}, {partial} partial)"
    );
}

/// **A backdrop covers the pixels its rectangle covered, each once, and
/// lays out no cell its target cannot hold.** The frame's own path is
/// followed: `PixelRect::covered_by` at the camera's ratio, cut to the
/// target (`PixelRect::within`), and the text areas `backdrop::text_areas`
/// hands glyphon for a shaped cell — each area's 64-pixel square glyph
/// clipped by **that area's own bounds**, and by the target, as glyphon
/// clips and clamps. For rectangles at fractional edges, `.5` ties,
/// ratios of 1, 1.5, 2 and 4, and targets that cut the rectangle on any
/// side, against a reference written from the rasterization rule itself
/// — a pixel is covered when its center lies in `[x0, x1)` and it is in
/// the target — checked pixel by pixel: the cells cover each covered
/// pixel exactly once and nothing else, `cell_count` is how many text
/// areas there are, and a target `W × H` holds at most `⌈W / 64⌉ ×
/// ⌈H / 64⌉` of them.
///
/// Controls: rounding the edges to the nearest pixel, or truncating
/// them, disagrees with the reference on some rectangle below — the
/// last loop asserts both — so the agreement is not a reference that
/// cannot tell them apart. Text areas without bounds would cover the
/// grid's overhang past the rectangle's right and bottom edges, and a
/// rectangle not cut to its target would lay out cells past the bound,
/// on the overhanging cases.
#[test]
fn test_a_backdrop_covers_the_pixels_its_rectangle_did() {
    use super::backdrop::{text_areas, BackdropCell, PixelRect, CELL_PX};

    baumhard::font::fonts::init();
    let cell = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (backdrop coverage)");
        BackdropCell::shape(&mut fs)
    };
    let rects = [
        (0.0_f32, 0.0_f32, 10.0_f32, 10.0_f32),
        (8.0, 597.25, 1264.0, 194.75),
        (10.5, 20.5, 129.0, 70.5),
        (0.25, 0.75, 63.5, 64.5),
        (100.4, 3.6, 200.2, 130.1),
        (-5.0, -5.0, 20.0, 20.0),
        (3.0, 3.0, 0.0, 9.0),
    ];
    // Every rectangle at every ratio, in a target that holds it, then
    // the cases a target cuts: the harness console's backdrop captured
    // at `scale = 4` into a 64-pixel image, a picker-sized rectangle
    // overhanging a small target on all four sides, and one wholly
    // outside its target.
    // A window-pixel rectangle, the camera's ratio, and the target's size.
    type Case = ((f32, f32, f32, f32), f32, (i32, i32));
    let mut cases: Vec<Case> = Vec::new();
    for rect in rects {
        for ratio in [1.0_f32, 1.5, 2.0] {
            cases.push((rect, ratio, (4000, 3000)));
        }
    }
    cases.push(((8.0, 597.25, 1264.0, 194.75), 4.0, (64, 64)));
    cases.push(((-100.0, -50.0, 2000.0, 1000.0), 1.5, (640, 480)));
    cases.push(((-100.0, -50.0, 2000.0, 1000.0), 1.0, (100, 70)));
    cases.push(((500.0, 500.0, 40.0, 40.0), 1.0, (300, 200)));
    // The reference: pixel `i` is covered when its center `i + 0.5` is
    // in `[from, to)`.
    let covered = |from: f32, to: f32, i: i32| {
        let center = f64::from(i) + 0.5;
        f64::from(from) <= center && center < f64::from(to)
    };
    let mut checked = 0;
    let mut cut = 0;
    for ((left, top, width, height), ratio, (target_w, target_h)) in cases {
        let (x0, x1) = (left * ratio, (left + width) * ratio);
        let (y0, y1) = (top * ratio, (top + height) * ratio);
        let rect = PixelRect::covered_by((left, top, width, height), ratio).within(target_w, target_h);
        let areas: Vec<glyphon::TextArea> = text_areas(&cell, rect).collect();
        assert_eq!(areas.len(), rect.cell_count(), "{rect:?}: the cell count");
        let most = ((target_w as f64 / 64.0).ceil() * (target_h as f64 / 64.0).ceil()) as usize;
        assert!(
            areas.len() <= most,
            "{rect:?} in a {target_w}×{target_h} target lays out {} cells, past the {most} it can hold",
            areas.len()
        );
        if x0 < 0.0 || y0 < 0.0 || x1 > target_w as f32 || y1 > target_h as f32 {
            cut += 1;
        }
        // How many cells hold each pixel of a margin around what the
        // target shows: each area's glyph, `CELL_PX` square at its
        // corner, clipped by the area's bounds and clamped to the
        // target, as glyphon draws it.
        let (from_x, to_x) = (-20, (x1.ceil() as i32).min(target_w) + 20);
        let (from_y, to_y) = (-20, (y1.ceil() as i32).min(target_h) + 20);
        let stride = (to_x - from_x) as usize;
        let mut holding = vec![0u8; stride * (to_y - from_y).max(0) as usize];
        for area in &areas {
            let (ax, ay) = (area.left as i32, area.top as i32);
            let (bounds_x, bounds_y) = (
                area.bounds.left.max(0)..area.bounds.right.min(target_w),
                area.bounds.top.max(0)..area.bounds.bottom.min(target_h),
            );
            for y in ay.max(bounds_y.start)..(ay + CELL_PX).min(bounds_y.end) {
                for x in ax.max(bounds_x.start)..(ax + CELL_PX).min(bounds_x.end) {
                    if (from_x..to_x).contains(&x) && (from_y..to_y).contains(&y) {
                        holding[(y - from_y) as usize * stride + (x - from_x) as usize] += 1;
                    }
                }
            }
        }
        for y in from_y..to_y {
            for x in from_x..to_x {
                let in_target = (0..target_w).contains(&x) && (0..target_h).contains(&y);
                let reference = covered(x0, x1, x) && covered(y0, y1, y) && in_target;
                let inside = (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y);
                assert_eq!(
                    inside, reference,
                    "({left}, {top}, {width}, {height}) at {ratio} in {target_w}×{target_h}: pixel ({x}, {y})"
                );
                let held = holding[(y - from_y) as usize * stride + (x - from_x) as usize];
                assert_eq!(held, u8::from(reference), "pixel ({x}, {y}) of {rect:?}");
                checked += 1;
            }
        }
    }
    assert!(
        checked > 1_000_000 && cut >= 4,
        "precondition: the pixels are checked ({checked}), and targets cut rectangles ({cut})"
    );

    // The controls: the two plausible wrong snaps each miss the rule.
    type Snap = fn(f32) -> i32;
    let snaps: [(&str, Snap); 2] = [("round", |v| v.round() as i32), ("truncate", |v| v as i32)];
    for (name, snap) in snaps {
        let differs = rects.iter().any(|&(left, top, width, height)| {
            [1.0_f32, 1.5, 2.0].iter().any(|&ratio| {
                let rect = PixelRect::covered_by((left, top, width, height), ratio);
                (
                    snap(left * ratio),
                    snap((left + width) * ratio),
                    snap(top * ratio),
                    snap((top + height) * ratio),
                ) != (rect.left, rect.right, rect.top, rect.bottom)
            })
        });
        assert!(
            differs,
            "the control: snapping by {name} disagrees with the rule somewhere"
        );
    }
}

/// **A console keystroke reshapes the prompt line and nothing else.**
/// The real console tree, the real in-place mutator (which writes every
/// slot on every keystroke), and the overlay rebuild's per-area reuse
/// (`overlay_dispatch::reshape_overlays`): of the many areas, only the
/// one whose text changed is shaped again.
///
/// Controls: the first rebuild shapes every area (so the store starts
/// from nothing); a mutator that writes the state already on screen
/// shapes none; and the same keystroke against a store that forgot its
/// keys shapes every area again — what the rebuild cost before the
/// reuse, and what a key that missed would cost.
#[test]
fn test_console_keystroke_reshapes_only_the_prompt_line() {
    use super::console_pass::{
        build_console_overlay_mutator, build_console_overlay_tree, console_overlay_signature,
    };
    use super::overlay_dispatch::{reshape_overlays, OverlayShapes};
    use baumhard::gfx_structs::scene::Scene;

    baumhard::font::fonts::init();
    let mut before = sample_console_geometry();
    before.input = "anchor".into();
    before.cursor_grapheme = 6;
    let layout_before = compute_console_frame_layout(&before, 1280.0, 720.0);
    let mut after = sample_console_geometry();
    after.input = "anchors".into();
    after.cursor_grapheme = 7;
    let layout_after = compute_console_frame_layout(&after, 1280.0, 720.0);
    assert_eq!(
        console_overlay_signature(&layout_before),
        console_overlay_signature(&layout_after),
        "precondition: a keystroke keeps the structure, so it takes the in-place arm"
    );

    let tree = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (keystroke tree)");
        build_console_overlay_tree(&before, &layout_before, &mut fs)
    };
    let mut scene = Scene::new();
    let id = scene.insert(tree, 0, Vec2::ZERO);
    let mut shapes = OverlayShapes::default();
    reshape_overlays(&mut shapes, &scene, &[id]);
    let areas = shapes.shaped;
    assert!(
        areas > 5,
        "control: the first rebuild shapes every area ({areas})"
    );

    let same = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (keystroke same)");
        build_console_overlay_mutator(&before, &layout_before, &mut fs)
    };
    scene.apply_mutator(id, &same);
    reshape_overlays(&mut shapes, &scene, &[id]);
    assert_eq!(
        shapes.shaped, areas,
        "control: the state already on screen shapes nothing"
    );

    let keystroke = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (keystroke)");
        build_console_overlay_mutator(&after, &layout_after, &mut fs)
    };
    scene.apply_mutator(id, &keystroke);
    let drawn = shapes.elements.len();
    reshape_overlays(&mut shapes, &scene, &[id]);
    assert_eq!(
        shapes.shaped,
        areas + 1,
        "a keystroke reshapes the prompt line alone"
    );
    assert_eq!(
        shapes.elements.len(),
        drawn,
        "and every other area keeps its buffers"
    );

    let mut forgot = OverlayShapes::default();
    reshape_overlays(&mut forgot, &scene, &[id]);
    assert_eq!(
        forgot.shaped, areas,
        "control: without the keys every area is shaped"
    );
}

/// **Every input shaping reads defeats the overlay reuse, one at a
/// time, and touches only its own area.** Two haloed areas in one
/// overlay tree; each row changes one thing
/// `tree_walker::shape_one_element_into_buffers` reads — the text, a
/// region's color alone, the scale, the line height, either bound, the
/// alignment, the halo's color, its width, or the halo itself — on the
/// first area, and the rebuild must shape exactly that area again. The
/// second area is untouched in every row, so it stays reused: the count
/// moves by one, not two.
///
/// So a field dropped from `overlay_dispatch::reuse_key` fails its row:
/// the area keeps buffers shaped from what it no longer holds.
///
/// Controls: the first rebuild shapes both areas; a rebuild of an
/// unchanged tree shapes none; and a moved area — position is not
/// shaped, it is where the stamps go — keeps its buffers and has them
/// written at the new place.
#[test]
fn test_every_input_shaping_reads_reshapes_its_area_and_only_it() {
    use super::overlay_dispatch::{reshape_overlays, OverlayShapes};
    use baumhard::core::primitives::ColorFontRegions;
    use baumhard::engine::tree::Placement;
    use baumhard::gfx_structs::scene::Scene;
    use baumhard::gfx_structs::tree::GfxTree;

    baumhard::font::fonts::init();
    let halo = OutlineStyle {
        color: [0, 0, 0, 255],
        px: 2.0,
    };
    let area = |text: &str, y: f32| {
        let mut area = GlyphArea::new_with_str(text, 16.0, 18.0, Vec2::new(10.0, y), Vec2::new(120.0, 24.0));
        area.regions = ColorFontRegions::single_span(text.chars().count(), Some([1.0, 1.0, 1.0, 1.0]), None);
        area.outline = Some(halo);
        area
    };
    type Change = fn(&mut GlyphArea);
    let rows: [(&str, Change); 10] = [
        ("text", |a| a.text = "alphz".into()),
        ("a region's color alone", |a| {
            a.regions = ColorFontRegions::single_span(5, Some([1.0, 0.0, 0.0, 1.0]), None)
        }),
        ("scale", |a| a.scale.0 = 17.0),
        ("line height", |a| a.line_height.0 = 19.0),
        ("the width bound", |a| a.render_bounds.x.0 = 121.0),
        ("the height bound", |a| a.render_bounds.y.0 = 25.0),
        ("alignment", |a| a.align_center = true),
        ("the halo's color", |a| {
            a.outline = Some(OutlineStyle {
                color: [0, 0, 255, 255],
                px: 2.0,
            })
        }),
        ("the halo's width", |a| {
            a.outline = Some(OutlineStyle {
                color: [0, 0, 0, 255],
                px: 3.0,
            })
        }),
        ("the halo itself", |a| a.outline = None),
    ];

    let build = || {
        let mut tree = GfxTree::new_non_indexed();
        let first = tree
            .insert_child(
                tree.root(),
                GfxElement::new_area_non_indexed_with_id(area("alpha", 0.0), 1, 1),
                Placement::Last,
            )
            .expect("under the root");
        tree.insert_child(
            tree.root(),
            GfxElement::new_area_non_indexed_with_id(area("bravo", 40.0), 2, 2),
            Placement::Last,
        )
        .expect("under the root");
        let mut scene = Scene::new();
        let id = scene.insert(tree, 0, Vec2::ZERO);
        (scene, id, first)
    };

    for (what, change) in rows {
        let (mut scene, id, first) = build();
        let mut shapes = OverlayShapes::default();
        reshape_overlays(&mut shapes, &scene, &[id]);
        assert_eq!(
            shapes.shaped, 2,
            "{what}: control: the first rebuild shapes both areas"
        );
        let drawn = shapes.elements.len();
        reshape_overlays(&mut shapes, &scene, &[id]);
        assert_eq!(
            shapes.shaped, 2,
            "{what}: control: an unchanged tree shapes nothing"
        );

        scene
            .tree_mut(id)
            .expect("the tree is in the scene")
            .update(first, |element| {
                change(element.glyph_area_mut().expect("an area"))
            })
            .expect("the area is in the tree");
        reshape_overlays(&mut shapes, &scene, &[id]);
        assert_eq!(
            shapes.shaped, 3,
            "{what}: a change to it reshapes its area and only it (the other stays reused)"
        );
        if what != "the halo itself" {
            assert_eq!(
                shapes.elements.len(),
                drawn,
                "{what}: the stamps are the same in number"
            );
        }
    }

    let (mut scene, id, first) = build();
    let mut shapes = OverlayShapes::default();
    reshape_overlays(&mut shapes, &scene, &[id]);
    let before: Vec<(f32, f32)> = shapes.elements.iter().map(|b| b.pos).collect();
    scene
        .tree_mut(id)
        .expect("the tree is in the scene")
        .update(first, |element| {
            element
                .glyph_area_mut()
                .expect("an area")
                .set_position((15.0, 7.0));
        })
        .expect("the area is in the tree");
    reshape_overlays(&mut shapes, &scene, &[id]);
    assert_eq!(shapes.shaped, 2, "control: a moved area keeps its buffers");
    let after: Vec<(f32, f32)> = shapes.elements.iter().map(|b| b.pos).collect();
    assert_eq!(before.len(), after.len(), "control: the same stamps are drawn");
    for (slot, (was, now)) in before.iter().zip(&after).enumerate().take(9) {
        let moved = (now.0 - was.0, now.1 - was.1);
        assert!(
            (moved.0 - 5.0).abs() < 1e-4 && (moved.1 - 7.0).abs() < 1e-4,
            "control: stamp {slot} moved with its area: {moved:?}"
        );
    }
    assert_eq!(
        before[9..],
        after[9..],
        "control: the other area's stamps stayed put"
    );
}
