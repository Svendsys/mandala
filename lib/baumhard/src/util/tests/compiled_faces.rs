// SPDX-License-Identifier: MPL-2.0

//! Running a test's check again in a child process whose font system holds
//! the compiled faces alone (#387): `FONTCONFIG_FILE` naming a config of
//! one empty directory, so the check fails alike on every host — a host
//! face can draw what the compiled ones do not, or, as one here does, map
//! every C0 and C1 control and every private-use code point. A child that reports no font system fails the test on
//! every host. A target whose fontdb does not read `FONTCONFIG_FILE` makes
//! a child whose font system holds host faces, and the test skips with a
//! note; CI runs on Linux and sets `MANDALA_REQUIRE_FONTCONFIG_HOSTS`,
//! under which it fails instead. A test is its own child: it runs its
//! check where [`CHILD`] names its mode, and otherwise asks for a child
//! ([`on_compiled_faces`]) and reads what it reported.

use std::process::Command;

/// The variable a child reads: which check it makes.
pub(crate) const CHILD: &str = "BAUMHARD_COMPILED_FACES_CHILD";

/// The path libtest runs the test this is written in under, as `--exact`
/// takes it: its own, read from the type of a function inside it, so it
/// cannot drift from the test's name or its module's.
macro_rules! this_test {
    () => {{
        fn here() {}
        let path = std::any::type_name_of_val(&here);
        let path = path.strip_suffix("::here").expect("a function's path");
        path.split_once("::").expect("a path in the crate").1.to_string()
    }};
}

/// The lines a child of `test` (its path under the crate, [`this_test`])
/// reported in mode `mode`, from a font system of the compiled faces alone;
/// `None`, with a note, where this target cannot make one. Its first report
/// is how many host faces its font system holds: none, or the target reads
/// no `FONTCONFIG_FILE`. A child that made no such report ran no check, and
/// fails the test wherever it runs.
pub(crate) fn on_compiled_faces(test: &str, mode: &str) -> Option<Vec<String>> {
    let scratch = std::env::temp_dir().join(format!("baumhard_compiled_faces_{}_{mode}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let (empty, home) = (scratch.join("empty"), scratch.join("home"));
    for dir in [&empty, &home] {
        std::fs::create_dir_all(dir).expect("a scratch directory");
    }
    let conf = scratch.join("fonts.conf");
    std::fs::write(
        &conf,
        format!(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig><dir>{}</dir></fontconfig>\n",
            empty.display()
        ),
    )
    .expect("a fontconfig file");
    let out = Command::new(std::env::current_exe().expect("the test binary has a path"))
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD, mode)
        .env("FONTCONFIG_FILE", &conf)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .output()
        .expect("the test binary runs");
    let _ = std::fs::remove_dir_all(&scratch);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{mode}: {text}");
    // libtest prints the test's name on the line a report starts, so a
    // report is found within a line.
    let reports: Vec<String> = text
        .lines()
        .filter_map(|line| line.split_once("REPORT ").map(|(_, report)| report.to_string()))
        .collect();
    let hosts: usize = reports
        .first()
        .and_then(|first| first.strip_prefix("HOSTS "))
        .and_then(|hosts| hosts.parse().ok())
        .unwrap_or_else(|| {
            panic!("{test}: the child ran no check, and made no report of its font system: {text}")
        });
    if hosts == 0 {
        return Some(reports[1..].to_vec());
    }
    assert!(
        std::env::var_os("MANDALA_REQUIRE_FONTCONFIG_HOSTS").is_none(),
        "precondition: the child's font system holds the compiled faces alone, not {hosts} host faces"
    );
    crate::util::test_skip::note_skipped(&format!(
        "{test}: this target does not read FONTCONFIG_FILE, so no child holds the compiled faces alone"
    ));
    None
}

/// In a child: report how many host faces its font system holds, the
/// first report every child makes. Whether a face is the host's is read
/// from its source, as `test_flow_shape_a_glyph_no_face_draws_has_no_ink`
/// reads it.
pub(crate) fn report_hosts() {
    crate::font::fonts::init();
    let mut lease = crate::font::fonts::FontSystemLease::new("the notdef bound's child");
    let hosts = lease
        .fonts()
        .db()
        .faces()
        .filter(|face| !matches!(face.source, cosmic_text::fontdb::Source::Binary(_)))
        .count();
    println!("REPORT HOSTS {hosts}");
}

/// `count` code points no face of `fs`'s font system maps, none of which
/// the shaper hides or hands over as a space: from the C0 and C1 controls
/// but the tab and the line breaks, and the private use area, in that
/// order, read against every face's `cmap`. What a shaping lays as
/// `.notdef`, each its own on any host; on the compiled faces alone, a
/// child's, there are hundreds.
pub(crate) fn undrawn(fs: &mut crate::font::fonts::FontSystemLease, count: usize) -> Vec<char> {
    let ids: Vec<_> = fs.fonts().db().faces().map(|face| face.id).collect();
    let fonts: Vec<_> = ids
        .into_iter()
        .filter_map(|id| fs.fonts().get_font(id, crate::font::FontWeight::NORMAL))
        .collect();
    let candidates = (0x01u32..0x20)
        .chain(0x80..0xA0)
        .chain(0xE000..0xF900)
        .filter_map(char::from_u32)
        .filter(|&c| !matches!(c, '\t' | '\n' | '\r' | '\u{1C}' | '\u{1D}' | '\u{1E}' | '\u{85}'));
    candidates
        .filter(|&c| fonts.iter().all(|font| font.as_swash().charmap().map(c) == 0))
        .take(count)
        .collect()
}

pub(crate) use this_test;
