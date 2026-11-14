// SPDX-License-Identifier: MPL-2.0

//! `maptool` as a user runs it: the built binary, against the format's
//! fixtures (`lib/baumhard/src/document/format/fixtures/`) and the
//! converted testament map, held to its stdout, its stderr and its exit
//! status.

use std::path::{Path, PathBuf};
use std::process::Command;

use baumhard::util::doc_fixtures::repo_path;
use baumhard::util::test_temp::TempDir;

/// What one run printed and how it ended.
struct Run {
    status: i32,
    stdout: String,
    stderr: String,
}

/// Run the built `maptool` with `args`.
fn maptool(args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_maptool"))
        .args(args)
        .output()
        .expect("maptool runs");
    Run {
        status: output.status.code().expect("maptool exits with a status"),
        stdout: String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr is UTF-8"),
    }
}

/// The format fixture `name`.
fn fixture(name: &str) -> PathBuf {
    repo_path(&format!("lib/baumhard/src/document/format/fixtures/{name}.mandala.json"))
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
    let nodes = text.lines().filter(|line| line.starts_with("      \"id\": ")).count();
    assert!(nodes > 200, "precondition: the testament map has its nodes: {nodes}");
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
/// sections, a portal end, an end's label, an end with only a label.
#[test]
fn test_grep_names_each_element_of_the_portals_fixture() {
    let run = maptool(&["grep", arg(&fixture("portals")), "."]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "#1 sections[0]: source\n\
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
    assert_eq!(run.stdout, "#147 sections[0]: the city where the Ark of the Covenant \n");
}

/// No match exits 1 and says so; a pattern that is not a regex is a
/// command line that cannot run, exit 2.
#[test]
fn test_grep_exits_1_on_no_match_and_2_on_a_bad_regex() {
    let run = maptool(&["grep", arg(&testament()), "xyzzy-no-such-token"]);
    assert_eq!((run.status, run.stdout.as_str()), (1, ""));
    assert!(run.stderr.contains("grep: no matches for \"xyzzy-no-such-token\""), "{}", run.stderr);
    let run = maptool(&["grep", arg(&testament()), "[unclosed"]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("grep: invalid regex \"[unclosed\""), "{}", run.stderr);
}

// --- export -----------------------------------------------------------

/// The tree fixture as Markdown, to stdout and to a file alike.
#[test]
fn test_export_writes_the_tree_fixture_as_markdown() {
    let expected = "# In the beginning\nGenesis 1:1\n\n## the heaven\n\n### the firmament\n\n## the earth\n\n\
                    ## darkness upon the face of the deep\n\n# Day one\n\n";
    let run = maptool(&["export", arg(&fixture("tree"))]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(run.stdout, expected);
    let dir = TempDir::new("maptool-export");
    let out = dir.join("out.md");
    let run = maptool(&["export", arg(&fixture("tree")), arg(&out)]);
    assert_eq!((run.status, run.stdout.as_str()), (0, ""), "{}", run.stderr);
    assert_eq!(std::fs::read_to_string(&out).expect("the export was written"), expected);
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

/// Every format fixture and every converted map loads: exit 0 and the
/// `valid` line, with no warning on the ones that earn none. The
/// fixtures that earn warnings earn exactly those their content shows:
/// `frames`' frame `{}` and `starts`' ringless frames, and `full`'s
/// hidden node.
#[test]
fn test_verify_passes_every_fixture_and_converted_map() {
    let clean = [
        fixture("colors"),
        fixture("deep_leaf"),
        fixture("fits"),
        fixture("minimal"),
        fixture("portals"),
        fixture("tree"),
        testament(),
        repo_path("maps/theme_demo.mandala.json"),
        repo_path("maps/palette_cascade.mandala.json"),
        repo_path("maps/stress_long_edges.mandala.json"),
    ];
    for map in &clean {
        let run = maptool(&["verify", arg(map)]);
        assert_eq!(run.status, 0, "{}", run.stderr);
        assert_eq!(run.stdout, format!("{}: valid\n", arg(map)));
    }
    let run = maptool(&["verify", arg(&fixture("frames"))]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "warning: #1 frame: visible with no rings\n{}: valid, 1 warning(s)\n",
            arg(&fixture("frames"))
        )
    );
    let run = maptool(&["verify", arg(&fixture("full"))]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(
        run.stdout
            .starts_with("warning: #2: hidden, and no verb or presentation can reveal it\n"),
        "{}",
        run.stdout
    );
    let run = maptool(&["verify", arg(&fixture("starts"))]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert!(run.stdout.ends_with(": valid, 13 warning(s)\n"), "{}", run.stdout);
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
            arg(&map)
        )
    );
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
        run.stderr.contains("map.json: not a map; a map's file name ends in `.mandala.json`"),
        "{}",
        run.stderr
    );
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
/// each one span of the new text, the span's own link gone with its old
/// spans — and it loads again.
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
    assert!(run.stdout.contains("  #2 THE HEAVEN [link → #4]\n"), "{}", run.stdout);
}

/// `--dry-run` prints the change as a unified diff, one hunk per changed
/// line, and leaves the file's bytes as they were.
#[test]
fn test_apply_dry_run_prints_the_diff_and_writes_nothing() {
    let dir = TempDir::new("maptool-apply-dry-run");
    let map = copy(&dir, "tree");
    let before = std::fs::read(&map).unwrap();
    let run = maptool(&["apply", arg(&map), "--dry-run", "heaven|earth", "--", "tr", "a-z", "A-Z"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "--- a/{0}\n+++ b/{0}\n\
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
    assert!(run.stderr.contains("apply: no section matches \"xyzzy\""), "{}", run.stderr);
    let run = maptool(&["apply", arg(&map), "the", "-i", "--dry-run", "--", "cat"]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("apply: -i writes the map and --dry-run writes nothing"), "{}", run.stderr);
    let run = maptool(&["apply", arg(&map), "the", "cat"]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("apply: missing `--` before the command"), "{}", run.stderr);
    let run = maptool(&["apply", arg(&map), "the", "--"]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("apply: missing the command after `--`"), "{}", run.stderr);
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
    let run = maptool(&["convert", "--legacy", "a.json", "b.mandala.json"]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("maptool: unknown command: convert"), "{}", run.stderr);
    let run = maptool(&[]);
    assert_eq!(run.status, 2);
    assert!(run.stderr.contains("maptool: missing command"), "{}", run.stderr);
}
