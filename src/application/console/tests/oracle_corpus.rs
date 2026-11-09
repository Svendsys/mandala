// SPDX-License-Identifier: MPL-2.0

//! The corpus the differential oracle runs.

use super::oracle::Sel;

/// `(selection fixture, input line)` — every line is executed and
/// its outcome compared against the pinned signature.
pub const EXEC_CORPUS: &[(Sel, &str)] = &[
    // help
    (Sel::Node, "help"),
    (Sel::Node, "help all"),
    (Sel::Node, "help color"),
    (Sel::Node, "help font"),
    (Sel::Node, "help nope"),
    // `help` declares no keys and one slot. These four used to
    // print as though the kv or the second word were not typed —
    // the full listing, then `color`'s usage three times — because
    // `help` was the one verb whose execute never called
    // `kvs::read_strict` (#156). A kv in the slot, after it, ahead
    // of it, and a second positional.
    (Sel::Node, "help color=1"),
    (Sel::Node, "help color section=2"),
    (Sel::Node, "help bg=x color"),
    (Sel::Node, "help color extra"),
    // fps
    (Sel::Node, "fps"),
    (Sel::Node, "fps on"),
    (Sel::Node, "fps off"),
    (Sel::Node, "fps debug"),
    (Sel::Node, "fps nope"),
    // mode
    (Sel::Node, "mode"),
    (Sel::Node, "mode default"),
    (Sel::Node, "mode resize"),
    (Sel::Node, "mode node-edit"),
    (Sel::Node, "mode nope"),
    // open / new / save
    (Sel::Node, "open"),
    // The same count, one slot along: `open` declares a path and
    // refuses a second one instead of loading the first in silence.
    (Sel::Node, "open a.mandala.json b.mandala.json"),
    (Sel::Node, "save"),
    (Sel::Node, "new"),
    (Sel::Node, "help ALL"),
    (Sel::Node, "help BORDER"),
    (Sel::Node, "fps ON"),
    (Sel::Node, "mode DEFAULT"),
    // screenshot — the verb only reads the line; a well-formed one is a
    // silent `OK` whose whole behavior is the side effect the signature
    // drops (the engine runs in the executor, against a renderer).
    (Sel::None, "screenshot"),
    (
        Sel::None,
        "screenshot shot.png width=640 height=360 scale=2 frame=fit sidecar=off",
    ),
    (Sel::None, "screenshot width=640"),
    (Sel::None, "screenshot frame=1,2,3"),
    (Sel::None, "screenshot space=surface"),
    (Sel::None, "screenshot overwrite=yes"),
    (Sel::None, "screenshot a.png b.png"),
    (Sel::None, "screenshot region=0,0,1,1"),
    // record — the verb only reads the line, so a well-formed one is a
    // silent OK and a malformed one is refused with the reason.
    (Sel::None, "record"),
    (Sel::None, "record start"),
    (
        Sel::None,
        "record start dir=frames fps=12 max_frames=240 width=640 height=360 frame=fit",
    ),
    (Sel::None, "record stop"),
    (Sel::None, "record pause"),
    (Sel::None, "record start fps=fast"),
    (Sel::None, "record start width=640"),
    (Sel::None, "record start space=surface"),
    (Sel::None, "record stop fps=30"),
    // clock — the verb only reads the line, so a well-formed one is a
    // silent OK (the switch and the step run in the executor) and a
    // malformed one is refused with the reason, at IPC's bounds.
    (Sel::None, "clock"),
    (Sel::None, "clock status"),
    (Sel::None, "clock virtual"),
    (Sel::None, "clock real"),
    (Sel::None, "clock step ms=500"),
    (Sel::None, "clock step frames=30"),
    (Sel::None, "clock step"),
    (Sel::None, "clock step ms=16 frames=1"),
    (Sel::None, "clock step ms=0"),
    (Sel::None, "clock step frames=fast"),
    (Sel::None, "clock pause"),
    (Sel::None, "clock real ms=5"),
    // The verbs an owner rebuilds on the one writer refuse, naming it
    // (`commands::rebuilt_by`); their grammars stand, and the
    // completion corpus below pins those.
    (Sel::Node, "node x=1"),
    (Sel::TwoSectionNode, "section x=1"),
    (Sel::Node, "color x=1"),
    (Sel::Node, "font x=1"),
    (Sel::Node, "zoom x=1"),
    (Sel::Path, "anchor x=1"),
    (Sel::Path, "body x=1"),
    (Sel::Path, "cap x=1"),
    (Sel::Path, "label x=1"),
    (Sel::Path, "spacing x=1"),
    // A multi-selection reaches the same refusals: the owner, not the
    // selection, decides them.
    (Sel::Multi, "node x=1"),
    (Sel::Multi, "mode resize"),
];

/// `(selection fixture, input line)` — the completion popup for
/// each line (cursor at end of line) is compared against the
/// pinned signature.
pub const COMPLETION_CORPUS: &[(Sel, &str)] = &[
    (Sel::Node, ""),
    (Sel::Node, "co"),
    (Sel::Path, "b"),
    (Sel::Node, "help "),
    (Sel::Node, "help co"),
    (Sel::Node, "help a"),
    (Sel::Path, "anchor "),
    (Sel::Path, "anchor fr"),
    (Sel::Path, "anchor from="),
    (Sel::Path, "anchor from=t"),
    (Sel::Path, "body "),
    (Sel::Path, "body glyph="),
    (Sel::Path, "body glyph=d"),
    (Sel::Path, "cap "),
    (Sel::Path, "cap from="),
    (Sel::Node, "color "),
    (Sel::Node, "color bg="),
    (Sel::Node, "color picker "),
    (Sel::Node, "color se"),
    (Sel::Node, "color ra"),
    (Sel::TwoSectionNode, "color range="),
    (Sel::Node, "font "),
    (Sel::Node, "font se"),
    (Sel::Node, "font size="),
    (Sel::TwoSectionNode, "font section="),
    (Sel::Node, "font ra"),
    (Sel::TwoSectionNode, "font range="),
    (Sel::TwoSectionNode, "color section="),
    (Sel::Node, "fps "),
    (Sel::Node, "fps o"),
    (Sel::Path, "spacing "),
    (Sel::Path, "spacing value="),
    (Sel::Path, "label "),
    (Sel::Path, "label position="),
    (Sel::Path, "label text="),
    (Sel::Node, "mode "),
    (Sel::Node, "mode d"),
    (Sel::Node, "help bg=x "),
    (Sel::Node, "help x="),
    (Sel::Node, "node "),
    (Sel::Node, "node r"),
    (Sel::Node, "open "),
    (Sel::Node, "save "),
    (Sel::Node, "new "),
    (Sel::TwoSectionNode, "section "),
    (Sel::TwoSectionNode, "section m"),
    (Sel::TwoSectionNode, "section move "),
    (Sel::TwoSectionNode, "section resize "),
    // The popup half of the same defect: `fill` is behind the
    // cursor, so the only form still in play is the one that reads
    // `section=` alone — and the two size keys are offered anyway.
    (Sel::TwoSectionNode, "section resize fill "),
    (Sel::TwoSectionNode, "section text "),
    (Sel::TwoSectionNode, "section add "),
    (Sel::TwoSectionNode, "section split "),
    (Sel::TwoSectionNode, "section show "),
    (Sel::TwoSectionNode, "section delete "),
    (Sel::TwoSectionNode, "section edit "),
    (Sel::TwoSectionNode, "section show section="),
    (Sel::TwoSectionNode, "section text runs="),
    (Sel::TwoSectionNode, "section frame "),
    (Sel::TwoSectionNode, "section frame preset="),
    (Sel::TwoSectionNode, "section frame preview "),
    (Sel::TwoSectionNode, "section frame preset "),
    (Sel::TwoSectionNode, "section frame se"),
    (Sel::TwoSectionNode, "section frame section="),
    (Sel::TwoSectionNode, "section frame preview section="),
    (Sel::Node, "zoom "),
    (Sel::Node, "zoom c"),
    (Sel::Node, "zoom min="),
    (Sel::Node, "zoom max="),
    (Sel::Node, "color PICKER "),
    (Sel::Node, "font SE"),
    (Sel::Node, "fps O"),
    (Sel::Path, "label CL"),
    (Sel::Node, "mode D"),
    (Sel::Node, "node R"),
    (Sel::TwoSectionNode, "section FRAME "),
    (Sel::TwoSectionNode, "section frame PREVIEW "),
    (Sel::Node, "help AL"),
    (Sel::Node, "zoom C"),
    // The exact half: a kv key filters case-sensitively because the
    // verb matches it that way, so an upper-case key offers nothing
    // rather than offering a row the verb would reject.
    // screenshot
    (Sel::None, "screenshot "),
    (Sel::None, "screenshot frame="),
    (Sel::None, "screenshot sidecar="),
    // record
    (Sel::None, "re"),
    (Sel::None, "record "),
    (Sel::None, "record start "),
    (Sel::None, "record start frame="),
    (Sel::None, "clock "),
    (Sel::None, "clock step "),
    // font families: the compiled set, the same on every machine since
    // #228, so pinned like every other vocabulary — whole, and narrowed
    // to a family, to the default face, and to the script faces.
    (Sel::Node, "font set "),
    (Sel::Node, "font set Ga"),
    (Sel::Node, "font set in"),
    (Sel::Node, "font set no"),
];

/// The one outcome whose tail this machine chose rather than the
/// console — pinned as a prefix, with the host-dependent bytes
/// left off the end.
///
/// Rust's `Display` for `io::Error` goes through `libc::strerror_r`, so
/// the words after the path are glibc's and are
/// `LC_MESSAGES`-sensitive: the same `open` of the same missing
/// file reads "No such file or directory" under `LC_ALL=C` and
/// something else under a translated locale. Pinning through the
/// errno string would assert the tester's locale, not the
/// console's behavior. The prefix that stops short of it is not
/// vacuous — it is every byte the console itself contributes, so
/// it pins `open.rs`'s `"open: {message}"` wrapper and
/// `MindMapDocument::load`'s `"{path}: could not read: "` inside it,
/// and a change to either fails this row.
///
/// **Prefix matching is the weaker check, so exactly one row gets
/// it**: every other readout in [`EXEC_CORPUS`] is made of the
/// console's own bytes and is pinned whole.
pub const EXEC_PREFIX_CORPUS: &[(Sel, &str)] = &[(Sel::Node, "open /nonexistent-dir-xyz/x.mandala.json")];
