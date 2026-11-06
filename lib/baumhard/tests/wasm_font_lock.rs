// SPDX-License-Identifier: MPL-2.0

//! Runs baumhard's `FONT_SYSTEM` helpers on `wasm32-unknown-unknown`,
//! in a real WebAssembly engine (node), and fails if they trap.
//!
//! **Why this test exists, and why it is an exception.**
//! `TEST_CONVENTIONS.md` §T9 trusts the wasm32 *compile* in `test.sh`
//! to validate wasm-specific paths, and that holds for everything a
//! compiler can see. It could not see this: `acquire_font_system_write`
//! called `std::time::Instant::now()` before its first attempt, which
//! compiles for wasm32 and traps the first time it runs there — in the
//! browser, at font initialization, with no message. Only executing the
//! code finds that, so this one test executes it.
//!
//! It is deliberately narrow. It is not `wasm-bindgen-test`, there is
//! no browser, and it touches no DOM or GPU: it loads a `cdylib` that
//! calls the lock helpers and checks two things the browser depends on.
//! The uncontended path works and reads no clock, and every same-thread
//! re-entrancy case panics with its own message — rather than hanging,
//! or trapping for some unrelated reason. Beside them it runs TextFlow
//! (`baumhard::flow`, #239) under the guard — shaping a run and a box,
//! and the two placements that shape again, `Fit` and a `Once` filler —
//! because that is the shaping path every frame, connector and section
//! takes in the browser. And it shapes the ornament fallback there
//! (#297): a `✦` in a Galatia SIL span drawn in `Mandala Ornaments`, a
//! `🌸` in `Mandala Emoji`, and a `~` in a span that pins no face drawn
//! in `Mandala Ornaments` rather than the decorative face the browser's
//! search met first before (the one case of the three the fallback's
//! order decides there, so the one its control turns red).
//!
//! **It also runs the tree walks 100,000 levels deep** — the mutation
//! validator over a deep mutator tree, and every arena, engine and
//! document walk over a deep chain (`arena::tests::deep_tests`, V1-A6
//! #290; `engine::tests::deep_tests`, `document::tests::deep_tests`,
//! V1-A5 #282) — because design §20
//! rule 11 asks every tree walk to prove, under wasm32 debug, that depth
//! costs it no call stack, and this is the probe that runs code there.
//! Beside it, R13's acceptance for the file format: a map 100,000
//! nodes deep through `load`, `validate_subtree`, `check_savable` and
//! `save`, and back — and, as its negative control, the same chain
//! counted by a walk that recurses, which must exhaust the engine's
//! stack.
//!
//! **What it needs.** The `wasm32-unknown-unknown` target and `node`.
//! Without either, the test prints a note and passes, the same
//! conditional-locally, unconditional-in-CI arrangement as `test.sh`'s
//! wasm32 gate. Set `MANDALA_REQUIRE_WASM_PROBE` (CI does) to turn a
//! missing prerequisite into a failure instead.
//!
//! **How it builds.** `probe.rs` is copied into a staging crate under
//! `<target>/wasm_font_lock_<hash>/` (three small files: a generated
//! manifest, the probe, the workspace's `Cargo.lock`), and built for
//! wasm32 from there. That compiles the dependency graph the app ships
//! and needs no second lockfile in the repository to go stale. The
//! first run compiles baumhard for wasm32 from scratch, which takes a
//! couple of minutes; later runs are incremental. `<hash>` is a short
//! hash of the workspace root, carried by the staging directory and by
//! the probe crate's name (`probe_names`), so two checkouts that share
//! one target directory each get their own staged manifest and their
//! own `.wasm` instead of rewriting each other's between a write and a
//! build, or between a build and the run.
//!
//! **Where it builds: the workspace's own target directory** (#285),
//! not a private one. `<target>` is the directory cargo built this
//! test in: `env!("CARGO_TARGET_TMPDIR")` is `<target>/tmp`, so its
//! parent is the directory cargo actually used, whether it came from
//! `$CARGO_TARGET_DIR` (relative or not), `build.target-dir`,
//! `CARGO_BUILD_TARGET_DIR`, `--target-dir`, or a wrapper that sets
//! its own, as `cargo llvm-cov` does for its instrumented build. The
//! build output goes to `<target>/` itself, so most of the probe's
//! host-side artifacts (proc macros and build scripts, about a fifth
//! of what the private directory held) are files the workspace's own
//! builds already made: `./test.sh`'s wasm32 check and clippy legs
//! build the host side in full even though their wasm32 output is
//! metadata only, and `cargo test` made some of the same. A handful
//! stay the probe's own, because their hash differs under the probe's
//! workspace root or features (baumhard's build script among them).
//! The wasm32 output sits in the `wasm32-unknown-unknown/` tree beside
//! the clippy, doc and check legs'.
//!
//! Sharing the directory shares no *decision*, and the module this
//! test runs is kept current by two different mechanisms. The
//! dependency units are named by a hash of package, features, profile,
//! target and flags, so a unit built for another configuration lands
//! beside the probe's rather than being mistaken for it. The probe's
//! own `.wasm` is not: a cdylib's file name carries no hash, so
//! it has one fingerprint however it is configured, and that
//! fingerprint, which any change of source, dependency or flags
//! dirties, is what relinks it before it runs. Either way an edit to
//! baumhard (or to `probe.rs`, or to the manifest below) is recompiled
//! before the module is run, which is what lets this test fail on a
//! broken subject at all, and what the private directory relied on in
//! exactly the same way; the build's success is asserted before the
//! path is read, so a failed build cannot leave an old file to be
//! judged. What does *not* share: the check-only wasm32 artifacts are
//! `.rmeta` files with no object code, which a build for execution
//! cannot reuse, so the wasm32 dependency compiles stay the probe's
//! own.
//!
//! **A new wait.** The nested build now takes the same build-directory
//! lock as every other cargo process on this `target/` (rust-analyzer's
//! check, another terminal), so it can sit at "Blocking waiting for
//! file lock on build directory" until they finish. That is a wait,
//! not a hang. It cannot wait on its own parent: cargo releases the
//! build lock before it runs a test binary.

use baumhard::document::tests::format_memory_shapes::{check_pins, derive, Measured};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Set to make a missing prerequisite a failure rather than a skip.
const REQUIRE_ENV: &str = "MANDALA_REQUIRE_WASM_PROBE";
const TARGET: &str = "wasm32-unknown-unknown";
const PROBE_CRATE_PREFIX: &str = "wasm_font_lock_probe";
const STAGING_PREFIX: &str = "wasm_font_lock";

/// Environment the probe's nested `cargo build` must not inherit from
/// this test process; see where it is removed.
const INHERITED_HOST_FLAGS: &[&str] = &[
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "RUSTDOCFLAGS",
    "CARGO_ENCODED_RUSTDOCFLAGS",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "LLVM_PROFILE_FILE",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above lib/baumhard")
}

/// The workspace's build output directory, which the probe's build
/// shares (see the module docs): the parent of cargo's own
/// `CARGO_TARGET_TMPDIR` for this test, so it is the directory cargo
/// really used and nothing here re-derives cargo's configuration.
fn target_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("CARGO_TARGET_TMPDIR is <target dir>/tmp")
        .to_path_buf()
}

/// FNV-1a, 64 bit. Written out here rather than taken from
/// `DefaultHasher`, which promises no stable output across toolchains:
/// the names it feeds should not change under a toolchain bump.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

/// The staging directory's name and the probe crate's name for one
/// workspace root: each carries a short hash of the root, so two
/// checkouts sharing one target directory never share a staged manifest
/// or a `.wasm` (see the module docs).
fn probe_names(root: &Path) -> (String, String) {
    let hash = format!("{:08x}", fnv1a(root.to_string_lossy().as_bytes()) >> 32);
    (
        format!("{STAGING_PREFIX}_{hash}"),
        format!("{PROBE_CRATE_PREFIX}_{hash}"),
    )
}

/// A missing prerequisite: skip with a note, or fail when CI demands
/// that the test really run.
fn unavailable(what: &str) {
    if std::env::var_os(REQUIRE_ENV).is_some() {
        panic!("{what}, and {REQUIRE_ENV} is set, so this test may not skip");
    }
    // Past libtest's capture, so the note shows under `./test.sh`.
    baumhard::util::test_skip::note_skipped(&format!(
        "{what}. The wasm32 font-lock probe did not run, so a green result \
         here says nothing about the browser build. (Set {REQUIRE_ENV} to make this a failure.)"
    ));
}

fn wasm_target_installed() -> bool {
    Command::new("rustc")
        .args(["--print", "target-libdir", "--target", TARGET])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
        .is_some_and(|dir| dir.is_dir())
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Rewrite `path` only when its content changes, so an unchanged file
/// keeps its mtime and cargo has no reason to rebuild.
fn write_if_changed(path: &Path, content: &[u8]) {
    if fs::read(path).ok().as_deref() != Some(content) {
        fs::write(path, content).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
    }
}

#[test]
fn test_font_lock_helpers_run_on_wasm32() {
    if !wasm_target_installed() {
        return unavailable(&format!(
            "the {TARGET} target is not installed (rustup target add {TARGET})"
        ));
    }
    if !node_available() {
        return unavailable("`node` is not on PATH");
    }

    let root = workspace_root();
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wasm_font_lock");
    let target = target_dir();
    let (stage_name, probe_crate) = probe_names(&root);
    // Inside the target directory, so `cargo clean` removes it and it
    // is already git-ignored. Only the three staged files live here;
    // the build output does not.
    let stage = target.join(stage_name);
    fs::create_dir_all(&stage).expect("the staging directory must be creatable");

    // The staging crate. `[workspace]` keeps cargo from adopting it
    // into the real workspace or demanding that it be listed there.
    // `getrandom`'s `wasm_js` feature is the same ask the root
    // manifest makes: without it baumhard's `rand` edge has no backend
    // on wasm32 and the build fails.
    //
    // That workspace boundary also keeps the root manifest's `[profile.dev]`
    // from reaching this build (cargo reads profiles from the
    // workspace root only), so the lean setting is written into the
    // manifest too (#283): `debug = false`, for the probe and
    // baumhard both. The module's debug info is most of its size on
    // disk, and nothing reads it: the test judges the module by what
    // it prints and by its panic messages, and a wasm trap has no
    // host stack for a line table to symbolize.
    let manifest = format!(
        "[package]\n\
         name = \"{probe_crate}\"\n\
         version = \"0.0.0\"\n\
         edition = \"2021\"\n\
         \n\
         [lib]\n\
         path = \"probe.rs\"\n\
         crate-type = [\"cdylib\"]\n\
         \n\
         [dependencies]\n\
         baumhard = {{ path = {baumhard:?} }}\n\
         getrandom = {{ version = \"0.4.2\", features = [\"wasm_js\"] }}\n\
         \n\
         [workspace]\n\
         \n\
         [profile.dev]\n\
         debug = false\n",
        baumhard = root.join("lib/baumhard").display().to_string(),
    );
    write_if_changed(&stage.join("Cargo.toml"), manifest.as_bytes());
    write_if_changed(
        &stage.join("probe.rs"),
        &fs::read(here.join("probe.rs")).expect("probe.rs ships beside this test"),
    );
    // The app's lockfile, so the probe resolves the app's versions.
    // Cargo adds the probe's own package and prunes what it does not
    // use; the copy is refreshed each run for that reason.
    fs::copy(root.join("Cargo.lock"), stage.join("Cargo.lock"))
        .expect("the workspace Cargo.lock must be readable");

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut build = Command::new(&cargo);
    // The flags this test process was built with are the *host's*, and
    // a nested cargo would otherwise inherit them for a wasm32 build:
    // `cargo llvm-cov` (`./test.sh --coverage`) injects
    // `-C instrument-coverage`, for which wasm32 has no profiler
    // runtime, and a `-C target-cpu=native` names a CPU wasm32 has not
    // got. The probe is meant to build the way the browser bundle
    // does, so it gets none of them.
    for var in INHERITED_HOST_FLAGS {
        build.env_remove(var);
    }
    let build = build
        .args(["build", "--target", TARGET, "--manifest-path"])
        .arg(stage.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .output()
        .expect("cargo must be runnable");
    assert!(
        build.status.success(),
        "building the wasm32 probe failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let wasm = target.join(format!("{TARGET}/debug/{probe_crate}.wasm"));
    assert!(
        wasm.is_file(),
        "the build succeeded but {} is missing",
        wasm.display()
    );

    let run = Command::new("node")
        .arg(here.join("runner.mjs"))
        .arg(&wasm)
        .output()
        .expect("node must be runnable");
    let report = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "the font-lock helpers misbehave on {TARGET}:\n{report}{}",
        String::from_utf8_lossy(&run.stderr)
    );

    // wasm32's memory limits are pinned to what the maps cost here, by
    // the module's own memory: every map is within the model (each
    // scenario checked that), and the pins are what the maps cost,
    // rounded up.
    let measured: Vec<Measured> = report.lines().filter_map(Measured::parse).collect();
    for m in &measured {
        println!(
            "{}: {} bytes read, {} written, {} elements, peaks {} loading and {} saving",
            m.shape, m.text_in, m.text_out, m.elements, m.load, m.both
        );
    }
    let (per_byte, per_element) = derive(&measured);
    println!("wasm32: {per_byte:.2} bytes per byte of text, {per_element:.1} bytes per element beyond it");
    if let Err(why) = check_pins(&measured) {
        panic!("{why}\n{report}");
    }
}

/// Two checkouts that share one target directory get different staging
/// directories and different probe crates, and one checkout gets the
/// same ones every run (a name that changed per run would leave a
/// fresh staging directory and a fresh `.wasm` behind each time).
/// The negative control: a `probe_names` that ignored its root would
/// fail the first assertion.
#[test]
fn test_two_workspace_roots_get_different_probe_names() {
    let a = probe_names(Path::new("/work/checkout-a"));
    let b = probe_names(Path::new("/work/checkout-b"));
    assert_ne!(a.0, b.0, "the staging directory must differ per root");
    assert_ne!(
        a.1, b.1,
        "the probe crate (and so its .wasm) must differ per root"
    );
    assert_eq!(a, probe_names(Path::new("/work/checkout-a")));
    assert!(a.0.starts_with(STAGING_PREFIX) && a.1.starts_with(PROBE_CRATE_PREFIX));
    // A valid crate name: cargo refuses anything else.
    assert!(a.1.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
}
