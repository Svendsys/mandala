// SPDX-License-Identifier: MPL-2.0

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
    /// Control: the same snippet with the guard removed is run the
    /// same way and must exit non-zero. Without it, "exits 0" would
    /// be satisfied by a harness that never reproduced the shell
    /// options in the first place.
    #[test]
    fn test_the_test_count_cannot_kill_a_green_run() {
        use std::process::Command;

        let dir = baumhard::util::test_temp::TempDir::new("test-sh-count");
        let log = dir.join("suite.log");
        std::fs::write(&log, "nothing here resembles cargo's summary\n")
            .expect("seed the log with no matching lines");

        let assignment = total_assignment_from_test_sh();
        assert!(
            assignment.contains("|| true"),
            "the count pipeline must swallow a no-match grep: {assignment}"
        );

        let run = |snippet: &str| {
            Command::new("bash")
                .arg("-c")
                .arg(format!(
                    "set -euo pipefail\nTEST_LOG={}\n{snippet}\nprintf '%s' \"$TOTAL\"",
                    log.display()
                ))
                .output()
                .expect("bash must be available to run the extracted snippet")
        };

        let guarded = run(&assignment);
        assert!(
            guarded.status.success(),
            "the count must not end the run; stderr: {}",
            String::from_utf8_lossy(&guarded.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&guarded.stdout), "0");

        let unguarded = run(&assignment
            .replace(" || true; }", "; }")
            .replace("{ ", "")
            .replace(" }", ""));
        assert!(
            !unguarded.status.success(),
            "control: without the guard the same snippet must end the run, or this \
             harness is not reproducing the shell options the script uses"
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
    /// there; or when `copy-dir` comes back, since the directory
    /// walk would then be shipping the archive again while every
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

        let shipped: BTreeSet<String> = shell
            .lines()
            .filter(|line| line.contains(r#"rel="copy-file""#))
            .filter_map(|line| line.split("href=\"../maps/").nth(1))
            .filter_map(|rest| rest.split('"').next())
            .map(|name| name.to_string())
            .collect();

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
