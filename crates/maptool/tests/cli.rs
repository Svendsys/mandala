// SPDX-License-Identifier: MPL-2.0

//! `maptool` as a user runs it: the built binary, against the format's
//! fixtures (`lib/baumhard/src/document/format/fixtures/`) and the
//! converted testament map, held to its stdout, its stderr and its exit
//! status.

use std::path::{Path, PathBuf};
use std::process::Command;

use baumhard::util::doc_fixtures::repo_path;
use baumhard::util::log::tilde_path;
use baumhard::util::test_temp::TempDir;

/// What one run printed and how it ended.
struct Run {
    status: i32,
    stdout: String,
    stderr: String,
}

/// Run the built `maptool` with `args`.
fn maptool(args: &[&str]) -> Run {
    maptool_in_home(args, None)
}

/// Run the built `maptool` with `args`, and with `HOME` set to `home`
/// when one is given.
fn maptool_in_home(args: &[&str], home: Option<&Path>) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_maptool"));
    command.args(args);
    if let Some(home) = home {
        command.env("HOME", home);
    }
    let output = command.output().expect("maptool runs");
    Run {
        status: output.status.code().expect("maptool exits with a status"),
        stdout: String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    }
}

/// The format fixture `name`.
fn fixture(name: &str) -> PathBuf {
    repo_path(&format!(
        "lib/baumhard/src/document/format/fixtures/{name}.mandala.json"
    ))
}

/// The converted testament map.
fn testament() -> PathBuf {
    repo_path("maps/testament.mandala.json")
}

/// `path` as an argument.
fn arg(path: &Path) -> &str {
    path.to_str().expect("the path is UTF-8")
}

/// A copy of the fixture `name` in `dir`, for a test that writes.
fn copy(dir: &TempDir, name: &str) -> PathBuf {
    let to = dir.join(&format!("{name}.mandala.json"));
    std::fs::copy(fixture(name), &to).expect("the fixture copies");
    to
}

/// `text` written to `dir` as `name`.
fn write(dir: &TempDir, name: &str, text: &str) -> PathBuf {
    let to = dir.join(name);
    std::fs::write(&to, text).expect("the map is written");
    to
}

// --- show -------------------------------------------------------------

/// The tree fixture, line for line: depth by indentation, both sections
/// of node 1 with the separator, its frame and its portal, node 2's line
/// link.
#[test]
fn test_show_prints_the_tree_fixture() {
    let run = maptool(&["show", arg(&fixture("tree"))]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "#1 In the beginning │ Genesis 1:1 [frame] [portal → #6]\n\
         \x20 #2 the heaven [link → #4]\n\
         \x20   #5 the firmament\n\
         \x20 #3 the earth\n\
         \x20 #4 darkness upon the face of the deep\n\
         #6 Day one\n"
    );
}

/// Every node of the testament map is one line: as many lines as the
/// file has node blocks (each opens with its own `"id"` line), led by the
/// first node, with the second indented under it and its line break
/// shown as `⏎`.
#[test]
fn test_show_prints_every_testament_node_on_one_line() {
    let text = std::fs::read_to_string(testament()).expect("the testament map reads");
    let nodes = text
        .lines()
        .filter(|line| line.starts_with("      \"id\": "))
        .count();
    assert!(
        nodes > 200,
        "precondition: the testament map has its nodes: {nodes}"
    );
    let run = maptool(&["show", arg(&testament())]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(run.stdout.lines().count(), nodes);
    let mut lines = run.stdout.lines();
    assert_eq!(lines.next(), Some("#1 Lord God"));
    assert_eq!(lines.next(), Some("  #2 commanded the man⏎Genesis 2:16"));
}

/// Portals are marked once per link, toward the node each leads to.
#[test]
fn test_show_marks_each_portal() {
    let run = maptool(&["show", arg(&fixture("portals"))]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "#1 source [portal → #2] [portal → #3] [portal → #2]\n\
         \x20 #2 child shown as a portal [portal → #1]\n\
         #3 far away\n"
    );
}

// --- grep -------------------------------------------------------------

/// Every element of the portals fixture that holds text, by its path:
/// sections, a portal end's own `content`, a portal end, an end's label,
/// an end with only a label.
#[test]
fn test_grep_names_each_element_of_the_portals_fixture() {
    let run = maptool(&["grep", arg(&fixture("portals")), "."]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "#1 sections[0]: source\n\
         #1 links[1].ends[0].content: ◈\n\
         #1 links[1].ends[1]: target\n\
         #1 links[1].ends[1].label: back\n\
         #1 links[2].ends[0].label: only the label\n\
         #2 sections[0]: child shown as a portal\n\
         #2 links[0].ends[0]: up\n\
         #2 links[0].ends[1]: down\n\
         #3 sections[0]: far away\n"
    );
}

/// `-i` matches regardless of case, anywhere on the command line; the
/// same pattern without it matches only its own case.
#[test]
fn test_grep_matches_testament_lines_with_and_without_case() {
    let run = maptool(&["grep", "-i", arg(&testament()), "covenant"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "#48 sections[0]: their covenant was made \n\
         #147 sections[0]: the city where the Ark of the Covenant \n\
         #234 sections[0]:  The verb for sexual union, covenant intimacy, craft mastery. \n"
    );
    let run = maptool(&["grep", arg(&testament()), "Covenant"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "#147 sections[0]: the city where the Ark of the Covenant \n"
    );
}

/// No match exits 1 and says so; a pattern that is not a regex is a
/// command line that cannot run, exit 2.
#[test]
fn test_grep_exits_1_on_no_match_and_2_on_a_bad_regex() {
    let run = maptool(&["grep", arg(&testament()), "xyzzy-no-such-token"]);
    assert_eq!((run.status, run.stdout.as_str()), (1, ""));
    assert!(
        run.stderr
            .contains("grep: no matches for \"xyzzy-no-such-token\""),
        "{}",
        run.stderr
    );
    let run = maptool(&["grep", arg(&testament()), "[unclosed"]);
    assert_eq!(run.status, 2);
    assert!(
        run.stderr.contains("grep: invalid regex \"[unclosed\""),
        "{}",
        run.stderr
    );
}

// --- export -----------------------------------------------------------

/// The tree fixture as Markdown, to stdout and to a file alike.
#[test]
fn test_export_writes_the_tree_fixture_as_markdown() {
    let expected =
        "# In the beginning\nGenesis 1:1\n\n## the heaven\n\n### the firmament\n\n## the earth\n\n\
                    ## darkness upon the face of the deep\n\n# Day one\n\n";
    let run = maptool(&["export", arg(&fixture("tree"))]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(run.stdout, expected);
    let dir = TempDir::new("maptool-export");
    let out = dir.join("out.md");
    let run = maptool(&["export", arg(&fixture("tree")), arg(&out)]);
    assert_eq!((run.status, run.stdout.as_str()), (0, ""), "{}", run.stderr);
    assert_eq!(
        std::fs::read_to_string(&out).expect("the export was written"),
        expected
    );
}

/// The testament map's export opens with its first node and the second
/// node's two lines under a deeper heading.
#[test]
fn test_export_writes_the_testament_map() {
    let run = maptool(&["export", arg(&testament())]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .starts_with("# Lord God\n\n## commanded the man\nGenesis 2:16\n\n### "),
        "{}",
        run.stdout.chars().take(200).collect::<String>()
    );
}

// --- verify -----------------------------------------------------------

/// `path` as `maptool` names it on a `verify` line: home folded to `~`.
fn shown(path: &Path) -> String {
    tilde_path(path)
}

/// Every `.mandala.json` under `dir`, sorted.
fn maps_in(dir: &str) -> Vec<PathBuf> {
    let root = repo_path(dir);
    let mut out: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{} lists: {e}", root.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| arg(path).ends_with(".mandala.json"))
        .collect();
    out.sort();
    out
}

/// The `verify` warnings of the three fixtures that earn any, whole:
/// `frames`' frame `{}`, `full`'s hidden node, and `starts`' thirteen
/// ringless frames, one per node.
fn expected_warnings(name: &str) -> Option<String> {
    match name {
        "frames.mandala.json" => Some("warning: #1 frame: visible with no rings\n".to_string()),
        "full.mandala.json" => {
            Some("warning: #2: hidden, and no verb or presentation can reveal it\n".to_string())
        }
        "starts.mandala.json" => Some(
            (1..=13)
                .map(|n| format!("warning: #{n} frame: visible with no rings\n"))
                .collect(),
        ),
        _ => None,
    }
}

/// Every format fixture and every map in `maps/` and `maps/demos/` — read
/// off the disk, so a new one is covered the day it lands — loads: exit
/// 0, and stdout is exactly its warnings and its `valid` line. Three
/// fixtures earn warnings, pinned whole; every other map earns none.
#[test]
fn test_verify_passes_every_fixture_and_converted_map() {
    let fixtures = maps_in("lib/baumhard/src/document/format/fixtures");
    let mut maps = maps_in("maps");
    maps.extend(maps_in("maps/demos"));
    assert!(
        fixtures.len() >= 9 && maps.len() >= 8,
        "precondition: the directories hold their maps: {fixtures:?} {maps:?}"
    );
    let mut warned = 0;
    for map in fixtures.iter().chain(&maps) {
        let run = maptool(&["verify", arg(map)]);
        assert_eq!(run.status, 0, "{}: {}", arg(map), run.stderr);
        let in_fixtures = fixtures.contains(map);
        let name = map.file_name().and_then(|n| n.to_str()).expect("a file name");
        let expected = match expected_warnings(name).filter(|_| in_fixtures) {
            Some(warnings) => {
                warned += 1;
                let count = warnings.lines().count();
                format!("{warnings}{}: valid, {count} warning(s)\n", shown(map))
            }
            None => format!("{}: valid\n", shown(map)),
        };
        assert_eq!(run.stdout, expected, "{}", arg(map));
    }
    assert_eq!(warned, 3, "precondition: the three warning fixtures were checked");
}

/// One map earning all four warnings prints them in the file's order —
/// node 1's hidden, frame and portal, then the verb — and still exits 0.
#[test]
fn test_verify_prints_each_kind_of_warning() {
    let dir = TempDir::new("maptool-verify-warnings");
    let map = write(
        &dir,
        "warned.mandala.json",
        r#"{"format": 1, "name": "w",
            "defaults": {"portal_end": {"content": ""}},
            "verbs": [{"id": "used", "name": "Used"}, {"id": "unused", "name": "Unused"}],
            "nodes": [
              {"id": 1, "position": [0, 0], "hidden": true, "on": {"click": "used"},
               "frame": {}, "links": [{"to": 2, "kind": "portal"}]},
              {"id": 2, "position": [0, 0]}
            ]}"#,
    );
    let run = maptool(&["verify", arg(&map)]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "warning: #1: hidden, and no verb or presentation can reveal it\n\
             warning: #1 frame: visible with no rings\n\
             warning: #1 links[0]: a portal whose ends both have no content\n\
             warning: verbs[1] \"unused\": defined, and nothing refers to it\n\
             {}: valid, 4 warning(s)\n",
            shown(&map)
        )
    );
}

/// `verify` names the map one way on every line, success and refusal
/// alike: home folded to `~`.
#[test]
fn test_verify_names_the_map_with_home_folded_on_success_and_refusal() {
    let dir = TempDir::new("maptool-verify-home");
    let good = write(&dir, "good.mandala.json", r#"{"format": 1, "name": "g"}"#);
    let bad = write(&dir, "bad.mandala.json", r#"{"format": 1}"#);
    let run = maptool_in_home(&["verify", arg(&good)], Some(dir.path()));
    assert_eq!(
        (run.status, run.stdout.as_str()),
        (0, "~/good.mandala.json: valid\n"),
        "{}",
        run.stderr
    );
    let run = maptool_in_home(&["verify", arg(&bad)], Some(dir.path()));
    assert_eq!(run.status, 1);
    assert!(run.stderr.starts_with("~/bad.mandala.json: "), "{}", run.stderr);
}

/// A map the loader refuses exits 1, with the loader's message — its
/// JSON path first — behind the file's path, and prints nothing on
/// stdout. A file that is not a `.mandala.json` is refused before it is
/// read.
#[test]
fn test_verify_exits_1_with_the_loaders_path_on_a_refused_map() {
    let dir = TempDir::new("maptool-verify-refused");
    let map = write(
        &dir,
        "broken.mandala.json",
        r#"{"format": 1, "name": "b", "nodes": [
             {"id": 1, "position": [0, 0]},
             {"id": 2, "parent": 3, "position": [0, 0]}
           ]}"#,
    );
    let run = maptool(&["verify", arg(&map)]);
    assert_eq!((run.status, run.stdout.as_str()), (1, ""));
    assert!(
        run.stderr.contains("broken.mandala.json: nodes[1].parent: "),
        "{}",
        run.stderr
    );
    let other = write(&dir, "map.json", r#"{"format": 1, "name": "b"}"#);
    let run = maptool(&["verify", arg(&other)]);
    assert_eq!(run.status, 1);
    assert!(
        run.stderr
            .contains("map.json: not a map; a map's file name ends in `.mandala.json`"),
        "{}",
        run.stderr
    );
}

/// `export` refuses an output path that names a map, so a slip of the
/// command line cannot replace a map with Markdown, and writes nothing.
#[test]
fn test_export_refuses_to_write_over_a_map() {
    let dir = TempDir::new("maptool-export-map");
    let map = copy(&dir, "tree");
    let before = std::fs::read(&map).unwrap();
    let run = maptool(&["export", arg(&map), arg(&map)]);
    assert_eq!(run.status, 2);
    assert!(
        run.stderr
            .contains("names a map; the Markdown goes to a file of its own"),
        "{}",
        run.stderr
    );
    assert_eq!(std::fs::read(&map).unwrap(), before);
}

/// An `<out.md>` that is a link to a map is refused naming both, and the
/// map is not touched: the "not a map" rule holds for the file written.
///
/// Control: checking only the name typed lets the export through, and
/// the map holds Markdown.
#[cfg(unix)]
#[test]
fn test_export_refuses_an_output_that_links_to_a_map() {
    let dir = TempDir::new("maptool-export-link-to-map");
    let map = copy(&dir, "tree");
    let before = std::fs::read(&map).unwrap();
    let link = dir.join("out.md");
    std::os::unix::fs::symlink(&map, &link).unwrap();
    let run = maptool(&["export", arg(&map), arg(&link)]);
    assert_eq!(run.status, 2, "{}", run.stderr);
    assert!(
        run.stderr.contains("out.md: links to `") && run.stderr.contains("tree.mandala.json`, a map"),
        "{}",
        run.stderr
    );
    assert_eq!(std::fs::read(&map).unwrap(), before);
}

/// An `<out.md>` that is a symlink is written through to its file, and
/// stays a link: the same rule as a map's save, for a path named on
/// maptool's own command line. A new output file is simply created.
///
/// Control: publishing at the path as given (no `real_file`) refuses
/// the link, and the status assertion goes red.
#[cfg(unix)]
#[test]
fn test_export_writes_through_a_symlinked_output() {
    let dir = TempDir::new("maptool-export-link");
    let real = write(&dir, "real.md", "old");
    let link = dir.join("out.md");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let run = maptool(&["export", arg(&fixture("tree")), arg(&link)]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert!(
        std::fs::read_to_string(&real)
            .unwrap()
            .starts_with("# In the beginning\n"),
        "the link's file holds the export"
    );
    let fresh = dir.join("fresh.md");
    let run = maptool(&["export", arg(&fixture("tree")), arg(&fresh)]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(std::fs::read_to_string(&fresh)
        .unwrap()
        .starts_with("# In the beginning\n"));
}

// --- apply ------------------------------------------------------------

/// The tree fixture's lines, with the three sections `^the` matches in
/// upper case: what `apply '^the' -- tr a-z A-Z` makes of it.
fn upper_cased_tree() -> String {
    let text = std::fs::read_to_string(fixture("tree")).expect("the tree fixture reads");
    text.replace(r#""the heaven""#, r#""THE HEAVEN""#)
        .replace(
            r#"{"text": "the firmament", "link": {"verb": "reveal-days"}}"#,
            r#"{"text": "THE FIRMAMENT"}"#,
        )
        .replace(r#""the earth""#, r#""THE EARTH""#)
}

/// `-i` rewrites the matched sections in the file and prints nothing; the
/// file it writes is the fixture with exactly those sections changed —
/// each one span of the new text, `the firmament`'s link gone with the
/// span it was on — and it loads again.
#[test]
fn test_apply_in_place_rewrites_the_matched_sections_and_reloads() {
    let dir = TempDir::new("maptool-apply-in-place");
    let map = copy(&dir, "tree");
    let expected = upper_cased_tree();
    assert_ne!(
        expected,
        std::fs::read_to_string(&map).unwrap(),
        "precondition: the expectation differs from the fixture"
    );
    let run = maptool(&["apply", arg(&map), "^the", "-i", "--", "tr", "a-z", "A-Z"]);
    assert_eq!((run.status, run.stdout.as_str()), (0, ""), "{}", run.stderr);
    assert!(
        run.stderr.contains("apply: 3 of 3 matched section(s) changed"),
        "{}",
        run.stderr
    );
    assert_eq!(std::fs::read_to_string(&map).unwrap(), expected);
    let run = maptool(&["show", arg(&map)]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(
        run.stdout.contains("  #2 THE HEAVEN [link → #4]\n"),
        "{}",
        run.stdout
    );
}

/// `--dry-run` prints the change as a unified diff of the file on disk
/// against what `-i` would write, headed by the path as given (no
/// `a/` before an absolute one), one hunk per changed line, and leaves
/// the file's bytes as they were.
#[test]
fn test_apply_dry_run_prints_the_diff_and_writes_nothing() {
    let dir = TempDir::new("maptool-apply-dry-run");
    let map = copy(&dir, "tree");
    let before = std::fs::read(&map).unwrap();
    let run = maptool(&[
        "apply",
        arg(&map),
        "--dry-run",
        "heaven|earth",
        "--",
        "tr",
        "a-z",
        "A-Z",
    ]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "--- {0}\t\n+++ {0}\t\n\
             @@ -60 +60 @@\n\
             -      \"sections\": [{{\"spans\": [{{\"text\": \"the heaven\"}}]}}],\n\
             +      \"sections\": [{{\"spans\": [{{\"text\": \"THE HEAVEN\"}}]}}],\n\
             @@ -73 +73 @@\n\
             -      \"sections\": [{{\"spans\": [{{\"text\": \"the earth\"}}]}}],\n\
             +      \"sections\": [{{\"spans\": [{{\"text\": \"THE EARTH\"}}]}}],\n",
            arg(&map)
        )
    );
    assert_eq!(std::fs::read(&map).unwrap(), before, "--dry-run changed the file");
}

/// On a map not in the saver's layout — here the whole map on one line —
/// `--dry-run` shows what `-i` really does: the one line goes and the
/// saver's lines come, and applying the diff to the file gives exactly
/// what `-i` then writes.
#[test]
fn test_apply_dry_run_previews_the_write_on_a_map_in_another_layout() {
    let dir = TempDir::new("maptool-apply-layout");
    let one_line = r#"{"format": 1, "name": "m", "nodes": [{"id": 1, "position": [0, 0], "sections": [{"spans": [{"text": "low"}]}]}]}"#;
    let map = write(&dir, "m.mandala.json", &format!("{one_line}\n"));
    let run = maptool(&["apply", arg(&map), "low", "--dry-run", "--", "tr", "a-z", "A-Z"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let mut lines = run.stdout.lines();
    assert_eq!(lines.next(), Some(format!("--- {}\t", arg(&map)).as_str()));
    assert_eq!(lines.next(), Some(format!("+++ {}\t", arg(&map)).as_str()));
    let hunk = lines.next().unwrap_or("");
    assert!(hunk.starts_with("@@ -1 +1,"), "{}", run.stdout);
    assert_eq!(lines.next(), Some(format!("-{one_line}").as_str()));
    let added: String = lines
        .map(|line| format!("{}\n", line.strip_prefix('+').unwrap_or("NOT AN ADDITION")))
        .collect();
    let run = maptool(&["apply", arg(&map), "low", "-i", "--", "tr", "a-z", "A-Z"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let written = std::fs::read_to_string(&map).unwrap();
    assert!(
        written.lines().count() > 3,
        "precondition: -i rewrote the layout: {written}"
    );
    assert_eq!(added, written);
}

/// `cat` changes nothing, whatever the text ends in: a section ending in
/// a line break is not one character shorter on the way back, and on
/// the testament map, every section through `cat` is unchanged.
#[test]
fn test_apply_through_cat_changes_nothing() {
    let dir = TempDir::new("maptool-apply-cat");
    let map = write(
        &dir,
        "m.mandala.json",
        r#"{"format": 1, "name": "m", "nodes": [{"id": 1, "position": [0, 0], "sections": [{"spans": [{"text": "foo\n"}]}]}]}"#,
    );
    let run = maptool(&["apply", arg(&map), "foo", "--dry-run", "--", "cat"]);
    assert_eq!((run.status, run.stdout.as_str()), (0, ""), "{}", run.stderr);
    assert!(
        run.stderr.contains("apply: 0 of 1 matched section(s) changed"),
        "{}",
        run.stderr
    );
    let run = maptool(&["apply", arg(&testament()), "", "--dry-run", "--", "cat"]);
    assert_eq!((run.status, run.stdout.as_str()), (0, ""), "{}", run.stderr);
    assert!(run.stderr.contains("apply: 0 of "), "{}", run.stderr);
}

/// `(?i)` in the pattern matches regardless of case, since `-i` is the
/// write flag; what a succeeding command writes on stderr is passed on.
#[test]
fn test_apply_matches_case_insensitively_with_the_pattern_flag_and_passes_stderr_on() {
    let dir = TempDir::new("maptool-apply-case");
    let map = copy(&dir, "tree");
    let run = maptool(&[
        "apply",
        arg(&map),
        "(?i)^THE EARTH$",
        "--",
        "sh",
        "-c",
        "echo note-from-the-command >&2; tr a-z A-Z",
    ]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(
        run.stderr.contains("apply: 1 of 1 matched section(s) changed"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("note-from-the-command"), "{}", run.stderr);
    assert!(run.stdout.contains(r#""THE EARTH""#), "{}", run.stdout);
}

/// The reviewer's repro: `apply -i` on a link named like a map that
/// leads to a file that is not one is refused, naming both, before any
/// command runs, and the file it leads to is not written.
///
/// Control: checking the name typed alone lets `-i` write the map into
/// `notamap.txt`.
#[cfg(unix)]
#[test]
fn test_apply_in_place_refuses_a_link_to_a_file_that_is_not_a_map() {
    let dir = TempDir::new("maptool-apply-sneaky");
    let other = dir.join("notamap.txt");
    std::fs::copy(fixture("tree"), &other).expect("the fixture copies");
    let before = std::fs::read(&other).unwrap();
    let link = dir.join("sneaky.mandala.json");
    std::os::unix::fs::symlink(&other, &link).unwrap();
    let run = maptool(&["apply", arg(&link), "^the earth", "-i", "--", "tr", "a-z", "A-Z"]);
    assert_eq!(run.status, 1, "{}", run.stderr);
    assert!(
        run.stderr.contains("sneaky.mandala.json: links to `")
            && run.stderr.contains("notamap.txt`, which is not a map"),
        "{}",
        run.stderr
    );
    assert_eq!(std::fs::read(&other).unwrap(), before);
}

/// With neither flag the changed map goes to stdout — the same text `-i`
/// would write — and the file is left as it was.
#[test]
fn test_apply_prints_the_changed_map_without_a_flag() {
    let dir = TempDir::new("maptool-apply-stdout");
    let map = copy(&dir, "tree");
    let before = std::fs::read(&map).unwrap();
    let run = maptool(&["apply", arg(&map), "^the", "--", "tr", "a-z", "A-Z"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(run.stdout, upper_cased_tree());
    assert_eq!(std::fs::read(&map).unwrap(), before);
}

/// The new span takes the first span's style and not its link: the
/// testament map's title keeps its size, color and weight, and a section
/// of two differently styled spans, the first linked, becomes one span
/// in the first one's style with no link.
#[test]
fn test_apply_keeps_the_first_spans_style_and_drops_its_link() {
    let dir = TempDir::new("maptool-apply-style");
    let map = dir.join("testament.mandala.json");
    std::fs::copy(testament(), &map).expect("the testament map copies");
    let run = maptool(&["apply", arg(&map), "^Lord God$", "-i", "--", "tr", "a-z", "A-Z"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(&map).unwrap();
    assert!(
        text.contains(
            r##"      "sections": [{"spans": [{"text": "LORD GOD", "size": 74.0, "color": "#ffffff", "weight": "bold"}]}]"##
        ),
        "{}",
        text.lines().find(|line| line.contains("LORD GOD")).unwrap_or("no LORD GOD line")
    );
    let two = write(
        &dir,
        "two.mandala.json",
        r#"{"format": 1, "name": "t", "nodes": [{"id": 1, "position": [0, 0], "sections": [{"spans": [
             {"text": "one ", "italic": true, "link": {"url": "https://example.com"}},
             {"text": "two", "weight": "bold"}]}]}]}"#,
    );
    let run = maptool(&["apply", arg(&two), "one two", "-i", "--", "tr", "a-z", "A-Z"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(&two).unwrap();
    assert!(
        text.contains(r#"      "sections": [{"spans": [{"text": "ONE TWO", "italic": true}]}]"#),
        "{text}"
    );
}

/// A command whose output is empty leaves the section with no spans.
#[test]
fn test_apply_with_empty_output_leaves_no_spans() {
    let dir = TempDir::new("maptool-apply-empty");
    let map = copy(&dir, "tree");
    let run = maptool(&["apply", arg(&map), "^Day one$", "-i", "--", "true"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    let text = std::fs::read_to_string(&map).unwrap();
    assert!(
        text.contains("      \"position\": [0.0, 600.0],\n      \"sections\": [{}]\n"),
        "{text}"
    );
}

/// A command that fails exits 1 with its status, and nothing is written,
/// though an earlier section's command had succeeded.
#[test]
fn test_apply_writes_nothing_when_a_command_fails() {
    let dir = TempDir::new("maptool-apply-fails");
    let map = copy(&dir, "tree");
    let before = std::fs::read(&map).unwrap();
    let run = maptool(&[
        "apply",
        arg(&map),
        "-i",
        "^the",
        "--",
        "sh",
        "-c",
        "read line; [ \"$line\" = \"the heaven\" ] && echo up || exit 3",
    ]);
    assert_eq!(run.status, 1);
    assert!(run.stderr.contains("apply: `sh` exited with 3"), "{}", run.stderr);
    assert_eq!(std::fs::read(&map).unwrap(), before);
}

/// No matching section exits 1; `-i` with `--dry-run`, and a missing
/// `--` or command, are command lines that cannot run, exit 2.
#[test]
fn test_apply_refuses_no_match_and_bad_command_lines() {
    let dir = TempDir::new("maptool-apply-refusals");
    let map = copy(&dir, "tree");
    let run = maptool(&["apply", arg(&map), "xyzzy", "--", "cat"]);
    assert_eq!(run.status, 1);
    assert!(
        run.stderr.contains("apply: no section matches \"xyzzy\""),
        "{}",
        run.stderr
    );
    let run = maptool(&["apply", arg(&map), "the", "-i", "--dry-run", "--", "cat"]);
    assert_eq!(run.status, 2);
    assert!(
        run.stderr
            .contains("apply: -i writes the map and --dry-run writes nothing"),
        "{}",
        run.stderr
    );
    let run = maptool(&["apply", arg(&map), "the", "cat"]);
    assert_eq!(run.status, 2);
    assert!(
        run.stderr.contains("apply: missing `--` before the command"),
        "{}",
        run.stderr
    );
    let run = maptool(&["apply", arg(&map), "the", "--"]);
    assert_eq!(run.status, 2);
    assert!(
        run.stderr.contains("apply: missing the command after `--`"),
        "{}",
        run.stderr
    );
}

// --- the command line -------------------------------------------------

/// `--help` lists exactly the five verbs, each at the start of its line,
/// and `convert` — deleted with the old format — is an unknown command.
#[test]
fn test_help_lists_exactly_the_five_verbs() {
    let run = maptool(&["--help"]);
    assert_eq!(run.status, 0);
    let verbs: Vec<&str> = run
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter(|line| !line.starts_with(' '))
        .filter_map(|line| line.split(' ').next())
        .collect();
    assert_eq!(verbs, ["show", "grep", "export", "verify", "apply"]);
    // What the help owes a user of `apply` (amendment E1.1): how to match
    // regardless of case, that matching is per line, and that a command
    // is not time-bounded. Each sentence is whole in the help.
    let help = run.stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    for owed in [
        "to match case-insensitively, start the pattern with (?i)",
        "For each section with a line the regex <pattern> matches",
        "is waited for with no time limit",
        "the 'content' a connector, line link or portal end sets of its own",
        "a frame's or ring's 'sides' ornament; and a ring's 'vertical' and 'corners'",
        "Not searched: what the map's defaults set",
    ] {
        assert!(help.contains(owed), "--help says {owed:?}: {help}");
    }
    let run = maptool(&["convert", "--legacy", "a.json", "b.mandala.json"]);
    assert_eq!(run.status, 2);
    assert!(
        run.stderr.contains("maptool: unknown command: convert"),
        "{}",
        run.stderr
    );
    let run = maptool(&[]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("maptool: missing command"), "{}", run.stderr);
}
