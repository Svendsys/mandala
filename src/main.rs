// SPDX-License-Identifier: MPL-2.0

// CODE_CONVENTIONS §9 closes with "Bare `unwrap()` outside tests is
// a bug", and this is the half of that rule an editor can tell you
// about while you type. `util::unwrap_posture` is the other half —
// it reads the workspace's source text and fails `./test.sh`, which
// is a hard gate where clippy here is advisory. Two mechanisms
// rather than one because they disagree usefully: the lint sees
// post-expansion code the text scan cannot read, and the scan sees
// the `pub mod tests;` trees the lint has to be told about.
//
// The `cfg_attr` is what keeps the lint off test code. A
// `#[cfg(test)] mod` does not exist in the build where the lint is
// live, and in the build where it does exist the whole crate is
// allowed — so `unwrap()` stays the right spelling in a test and a
// bug everywhere else.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use crate::application::app::{Application, Options};
use crate::application::keybinds::KeybindConfig;
use log::info;

mod application;

const DEFAULT_MINDMAP: &str = "maps/testament.mindmap.json";

#[cfg(not(target_arch = "wasm32"))]
fn parse_cli() -> (String, Option<std::path::PathBuf>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mindmap_path: Option<String> = None;
    let mut keybinds_path: Option<std::path::PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--keybinds" {
            if let Some(val) = args.get(i + 1) {
                keybinds_path = Some(std::path::PathBuf::from(val));
                i += 2;
                continue;
            }
        } else if let Some(val) = a.strip_prefix("--keybinds=") {
            keybinds_path = Some(std::path::PathBuf::from(val));
        } else if !a.starts_with("--") && mindmap_path.is_none() {
            mindmap_path = Some(a.clone());
        }
        i += 1;
    }
    (
        mindmap_path.unwrap_or_else(|| DEFAULT_MINDMAP.to_string()),
        keybinds_path,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn create_options() -> Options {
    let (mindmap_path, keybinds_path) = parse_cli();
    let keybind_config = KeybindConfig::load_for_desktop(keybinds_path.as_deref());

    Options {
        should_exit: false,
        mindmap_path,
        keybind_config,
    }
}

#[cfg(target_arch = "wasm32")]
fn create_options() -> Options {
    // WASM: mindmap_path and keybind_config are replaced later by run_wasm.
    Options {
        should_exit: false,
        mindmap_path: DEFAULT_MINDMAP.to_string(),
        keybind_config: KeybindConfig::default(),
    }
}

fn main() {
    baumhard::util::log::init();
    #[cfg(not(target_arch = "wasm32"))]
    info!("Starting Mandala (native)");
    #[cfg(target_arch = "wasm32")]
    info!("Starting Mandala (WASM)");

    let app = Application::new(create_options());
    app.run();
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// The `TOTAL=` line in `test.sh` extracted verbatim, from its
    /// `TOTAL=$(` through the closing paren of the assignment.
    ///
    /// Read out of the script rather than restated here: a copy of
    /// the pipeline would go on passing after the real one changed,
    /// which is the mirror shape a test must not take.
    fn total_assignment_from_test_sh() -> String {
        let script = std::fs::read_to_string(repo_root().join("test.sh")).expect("test.sh must be readable");
        let start = script
            .find("TOTAL=$(")
            .expect("test.sh must still compute a test count into TOTAL");
        let mut depth = 0usize;
        for (offset, ch) in script[start..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return script[start..start + offset + 1].to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("the TOTAL assignment in test.sh has no closing paren");
    }

    /// **A test count cannot fail a green run.**
    ///
    /// `test.sh` runs under `set -euo pipefail`. A `grep` that
    /// matches nothing exits 1, the pipeline takes that status, the
    /// assignment takes the pipeline's, and `set -e` ends the script
    /// — after the suite has already passed, printing nothing. Any
    /// change to cargo's summary wording turns a green run into a
    /// silent non-zero exit that reads as a test failure.
    ///
    /// The assignment is lifted out of `test.sh` and executed under
    /// the same shell options against a log with no matching lines,
    /// which is the exact condition. It must exit 0 and produce `0`.
    ///
    /// Control, in **two rows**, because one row cannot tell a
    /// disabled mechanism from a broken snippet. The guard is
    /// removed and nothing else — `{ grep … || true; }` back to the
    /// bare `grep …` — and the result is run twice: under the
    /// script's shell options it must exit **1**, the status
    /// `pipefail` hands the assignment and `errexit` acts on; with
    /// those options off, the same text must exit 0 and still print
    /// `0`. Only the pair shows that the failure is the mechanism
    /// rather than something the edit did to the snippet.
    ///
    /// That is not hypothetical here. The first version of this
    /// control built its "unguarded" text with three replace-alls,
    /// two of which also rewrote the `awk` program (`{ sum += $4 }
    /// END { print sum+0 }` shares its braces with the guard), and
    /// the result was a bash *syntax error* — non-zero with the
    /// options and non-zero without them, so it never reached the
    /// `pipefail` path it claimed to be demonstrating. Issue #138's
    /// refinement, verbatim: the control has to disable the
    /// mechanism on the path the test exercises.
    #[test]
    fn test_the_test_count_cannot_kill_a_green_run() {
        use std::process::Command;

        /// The options `test.sh` sets, and the only difference
        /// between the control's two rows.
        const SHELL_OPTIONS: &str = "set -euo pipefail";

        let dir = baumhard::util::test_temp::TempDir::new("test-sh-count");
        let log = dir.join("suite.log");
        std::fs::write(&log, "nothing here resembles cargo's summary\n")
            .expect("seed the log with no matching lines");

        let assignment = total_assignment_from_test_sh();
        assert!(
            assignment.contains("|| true"),
            "the count pipeline must swallow a no-match grep: {assignment}"
        );

        let run = |options: &str, snippet: &str| {
            Command::new("bash")
                .arg("-c")
                .arg(format!(
                    "{options}\nTEST_LOG={}\n{snippet}\nprintf '%s' \"$TOTAL\"",
                    log.display()
                ))
                .output()
                .expect("bash must be available to run the extracted snippet")
        };

        let guarded = run(SHELL_OPTIONS, &assignment);
        assert!(
            guarded.status.success(),
            "the count must not end the run; stderr: {}",
            String::from_utf8_lossy(&guarded.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&guarded.stdout), "0");

        // Unwrap the group and drop the `|| true`, touching nothing
        // else — in particular not the `awk` program's braces.
        let unguarded = assignment.replace("{ grep", "grep").replace(" || true; }", "");
        assert!(
            unguarded != assignment && !unguarded.contains("|| true"),
            "the control must actually remove the guard; got {unguarded} from {assignment}"
        );

        let control = run(SHELL_OPTIONS, &unguarded);
        assert_eq!(
            control.status.code(),
            Some(1),
            "control: unguarded, the assignment must end the run with the status \
             `pipefail` gives it — an exit code that is not 1 means the snippet \
             broke for a reason of its own. stderr: {}",
            String::from_utf8_lossy(&control.stderr)
        );

        let mechanism_off = run("", &unguarded);
        assert!(
            mechanism_off.status.success(),
            "control: the same unguarded text with the shell options off must \
             succeed, or the row above proved nothing about `set -euo pipefail`. \
             stderr: {}",
            String::from_utf8_lossy(&mechanism_off.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&mechanism_off.stdout),
            "0",
            "with the options off the unguarded pipeline still computes the count; \
             only the exit status differs"
        );
    }

    /// Every `.mindmap.json` in `maps/` is named by a `copy-file`
    /// entry in the browser shell, and nothing else in that
    /// directory is.
    ///
    /// The shell used to ship the whole directory with `copy-dir
    /// ../maps`, which put `testament.mind` — a 1 MB 7z miMind
    /// archive no browser code can open — into every bundle. Naming
    /// the fixtures one by one fixes that and creates the opposite
    /// hazard: a fixture added to `maps/` and not to the shell is
    /// simply absent from the bundle, and `?map=` on it 404s with
    /// nothing at build time to say why. This is the check that
    /// closes both directions at once.
    ///
    /// Fails when: a `.mindmap.json` is added to `maps/` and not to
    /// `web/index.html`; when an entry names a file that is not
    /// there; when an entry loses its `data-target-path="maps"`,
    /// which is the attribute that reproduces the `dist/maps/`
    /// layout `?map=` and `DEFAULT_MINDMAP` are written against —
    /// without it trunk drops the fixture in the dist root and the
    /// URL 404s at runtime with nothing at build time to say why;
    /// or when `copy-dir` comes back, since the directory walk
    /// would then be shipping the archive again while every
    /// per-file assertion still passed.
    ///
    /// Reads the two sides independently — the filesystem and the
    /// HTML text — so neither can be derived from the other.
    #[test]
    fn test_every_shipped_map_reaches_the_browser_bundle() {
        let shell = std::fs::read_to_string(repo_root().join("web/index.html"))
            .expect("the browser shell must be readable");

        assert!(
            !shell.contains(r#"rel="copy-dir""#),
            "a `copy-dir` in the shell ships whatever is in the directory, including the              1 MB `.mind` archive; name the fixtures instead"
        );

        let on_disk: BTreeSet<String> = std::fs::read_dir(repo_root().join("maps"))
            .expect("maps/ must be readable")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".mindmap.json"))
            .collect();
        assert!(
            !on_disk.is_empty(),
            "no fixtures found in maps/, so this test would pass over an empty set"
        );

        let mut shipped: BTreeSet<String> = BTreeSet::new();
        for line in shell.lines().filter(|line| line.contains(r#"rel="copy-file""#)) {
            // The one half of the layout claim that is checkable
            // without running trunk. `copy-file` takes exactly one
            // optional attribute, and it is this one; the fixture
            // lands in the dist root without it.
            assert!(
                line.contains(r#"data-target-path="maps""#),
                "every shipped fixture must carry `data-target-path=\"maps\"` — it is what \
                 puts the file under `dist/maps/`, which is where `?map=` and \
                 `DEFAULT_MINDMAP` look for it: {line}"
            );
            let name = line
                .split(r#"href="../maps/"#)
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_else(|| panic!("a `copy-file` entry must name a file in `../maps/`: {line}"));
            shipped.insert(name.to_string());
        }

        assert_eq!(
            shipped, on_disk,
            "web/index.html and maps/ disagree about what the browser build ships"
        );

        // The default the browser falls back to when `?map=` is
        // absent has to be one of them, or a bare page load 404s.
        let default_name = super::DEFAULT_MINDMAP
            .rsplit('/')
            .next()
            .expect("DEFAULT_MINDMAP names a file");
        assert!(
            shipped.contains(default_name),
            "the default map {default_name:?} must be in the bundle; shipped: {shipped:?}"
        );
    }
}
