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
    // anchor
    (Sel::Edge, "anchor"),
    (Sel::Edge, "anchor from=top"),
    (Sel::Edge, "anchor from=top to=bottom"),
    (Sel::Edge, "anchor from=diagonal"),
    (Sel::Edge, "anchor bogus=1"),
    (Sel::Node, "anchor from=top"),
    // body
    (Sel::Edge, "body"),
    (Sel::Edge, "body glyph=dash"),
    (Sel::Edge, "body glyph=nope"),
    (Sel::Edge, "body bogus=1"),
    // border
    (Sel::Node, "border"),
    (Sel::Node, "border on"),
    (Sel::Node, "border off"),
    (Sel::Node, "border toggle"),
    (Sel::Node, "border show side=nope"),
    // The three readouts, pinned whole. They spent a while in
    // `EXEC_PREFIX_CORPUS` on the belief that their bodies were
    // measured off the host's fonts — see the reason recorded
    // there for why they are not, and what a prefix let through.
    (Sel::Node, "border show"),
    (Sel::Node, "border show verbose"),
    (Sel::Node, "border show side=top"),
    (Sel::Node, "border reset"),
    (Sel::Node, "border nope"),
    (Sel::Node, "border preset heavy"),
    (Sel::Node, "border preset nope"),
    (Sel::Node, "border preset cycle"),
    (Sel::Node, "border preset"),
    (Sel::Node, "border color accent"),
    (Sel::Node, "border color"),
    (Sel::Node, "border padding 4"),
    (Sel::Node, "border padding abc"),
    (Sel::Node, "border padding"),
    (Sel::Node, "border palette off"),
    (Sel::Node, "border palette"),
    (Sel::Node, "border font off"),
    (Sel::Node, "border font"),
    (Sel::Node, "border side top reset"),
    (Sel::Node, "border side nope reset"),
    (Sel::Node, "border side top"),
    (Sel::Node, "border side"),
    (Sel::Node, "border corner tl reset"),
    (Sel::Node, "border corner nope x"),
    (Sel::Node, "border corner tl"),
    (Sel::Node, "border corner"),
    (Sel::Node, "border preset=heavy"),
    (Sel::Node, "border size=12"),
    (Sel::Node, "border size=abc"),
    (Sel::Node, "border bogus=1"),
    (Sel::Node, "border preview preset=heavy"),
    (Sel::Node, "border preview commit"),
    (Sel::Node, "border preview cancel"),
    (Sel::Node, "border preview nope"),
    // The shared preview dispatcher normalizes the subverb to
    // match on it and used to quote the *normalized* spelling
    // back, so this line answered `unknown subverb 'nope'`. One
    // row per label the dispatcher wears, because the wording is
    // the only thing the four verbs do not each own.
    (Sel::Node, "border preview NOPE"),
    (Sel::Node, "border preview"),
    (Sel::Edge, "border on"),
    // The per-node twin of the canvas kv-form row below: seven of
    // the thirteen subverbs the token-0 popup used to offer are
    // refused at a slot a kv already made kv-form, and the other
    // six are not — each for its own reason, which is why the
    // popup withholds only the seven.
    (Sel::Node, "border color=#fff preset heavy"),
    (Sel::Node, "border color=#fff preview"),
    (Sel::Node, "border color=#fff show side=nope"),
    (Sel::Node, "border color=#fff on"),
    // The discriminator reads *position*, and three surfaces asked
    // "is there a kv anywhere" instead. A kv past the subverb slot
    // leaves the slot positional, so each of these is an unknown
    // subverb; each used to answer with the quoting hint naming a
    // word that is not a palette.
    (Sel::Node, "border nope palette=coral"),
    (Sel::Node, "border preview nope palette=coral"),
    (Sel::Node, "canvas border preview nope palette=coral"),
    // …and the lines the hint is actually for, on the two surfaces
    // that now reach it through the shared wording rather than a
    // near-copy of it.
    (Sel::Node, "border preview palette=My Palette"),
    // canvas
    (Sel::Node, "canvas"),
    (Sel::Node, "canvas nope"),
    (Sel::Node, "canvas border"),
    (Sel::Node, "canvas border show"),
    (Sel::Node, "canvas border reset"),
    (Sel::Node, "canvas border preset heavy"),
    (Sel::Node, "canvas border preset nope"),
    (Sel::Node, "canvas border nope"),
    (Sel::Node, "canvas border bogus=1"),
    (Sel::Node, "canvas border preset=heavy"),
    (Sel::Node, "canvas border padding abc"),
    (Sel::Node, "canvas border side top reset"),
    (Sel::Node, "canvas border corner tl reset"),
    (Sel::Node, "canvas border corner nope x"),
    // A kv ahead of the subverb slot means kv form, on the canvas
    // surfaces exactly as on the per-node one — this dispatched
    // positionally and dropped the color without a word.
    (Sel::Node, "canvas border color=#fff preset heavy"),
    // …and the three the discriminator does *not* gate, on both
    // subjects and past the `focused` modifier. These are why the
    // popup withholds seven of ten rather than all ten.
    (Sel::Node, "canvas border color=#fff show"),
    (Sel::Node, "canvas border color=#fff reset"),
    (Sel::Node, "canvas border color=#fff preview commit"),
    (Sel::Node, "canvas section-frame color=#fff focused preset heavy"),
    (Sel::Node, "canvas section-frame color=#fff focused show"),
    (Sel::Node, "canvas section-frame"),
    (Sel::Node, "canvas section-frame show"),
    (Sel::Node, "canvas section-frame reset"),
    (Sel::Node, "canvas section-frame nope"),
    (Sel::Node, "canvas section-frame focused show"),
    (Sel::Node, "canvas section-frame focused reset"),
    (Sel::Node, "canvas section-frame focused nope"),
    (Sel::Node, "canvas section-frame side top reset"),
    (Sel::Node, "canvas section-frame focused side top reset"),
    (Sel::Node, "canvas border preview commit"),
    (Sel::Node, "canvas border preview cancel"),
    (Sel::Node, "canvas border preview nope"),
    (Sel::Node, "canvas border preview NOPE"),
    // cap
    (Sel::Edge, "cap"),
    (Sel::Edge, "cap from=arrow"),
    (Sel::Edge, "cap from=arrow to=none"),
    (Sel::Edge, "cap from=nope"),
    (Sel::Edge, "cap bogus=1"),
    // color
    (Sel::Node, "color"),
    (Sel::Node, "color bg=#112233"),
    (Sel::Node, "color bg=nope"),
    (Sel::Node, "color bogus=1"),
    (Sel::Node, "color pick"),
    (Sel::Node, "color bg"),
    (Sel::Node, "color text"),
    (Sel::Node, "color border"),
    (Sel::Node, "color picker on"),
    (Sel::Node, "color picker off"),
    (Sel::Node, "color picker"),
    (Sel::Node, "color picker nope"),
    (Sel::Node, "color nope"),
    (Sel::Section, "color bg"),
    (Sel::Node, "color section=0 bg=#112233"),
    (Sel::Node, "color section=abc bg=#112233"),
    (Sel::TwoSectionNode, "color section=0 range=0..2 text=accent"),
    (Sel::TwoSectionNode, "color range=0..2 text=accent"),
    (Sel::TwoSectionNode, "color section=0 range=zz text=accent"),
    // edge
    (Sel::Edge, "edge"),
    (Sel::Edge, "edge type=cross_link"),
    (Sel::Edge, "edge type=nope"),
    (Sel::Edge, "edge reset=straight"),
    (Sel::Edge, "edge reset=nope"),
    (Sel::Edge, "edge display_mode=portal"),
    (Sel::Edge, "edge display_mode=nope"),
    (Sel::Edge, "edge bogus=1"),
    // font
    (Sel::Node, "font"),
    (Sel::Node, "font set"),
    (Sel::Node, "font set NoSuchFamilyHere"),
    (Sel::Node, "font nope"),
    (Sel::Node, "font size=14"),
    (Sel::Node, "font size=abc"),
    (Sel::Node, "font size=0"),
    (Sel::Node, "font bogus=1"),
    (Sel::Edge, "font min=8 max=4"),
    (Sel::Node, "font range=0..2"),
    (Sel::TwoSectionNode, "font section=0 range=0..2 size=12"),
    (Sel::TwoSectionNode, "font section=0 range=zz size=12"),
    (Sel::TwoSectionNode, "font section=abc size=12"),
    (Sel::TwoSectionNode, "font section=0 min=8"),
    // fps
    (Sel::Node, "fps"),
    (Sel::Node, "fps on"),
    (Sel::Node, "fps off"),
    (Sel::Node, "fps debug"),
    (Sel::Node, "fps nope"),
    // spacing
    (Sel::Edge, "spacing"),
    (Sel::Edge, "spacing value=tight"),
    (Sel::Edge, "spacing value=3.5"),
    (Sel::Edge, "spacing value=nope"),
    (Sel::Edge, "spacing bogus=1"),
    // label
    (Sel::EdgeLabel, "label"),
    (Sel::EdgeLabel, "label clear"),
    (Sel::EdgeLabel, "label text=hi"),
    (Sel::EdgeLabel, "label text=hi position=start"),
    (Sel::EdgeLabel, "label position=nope"),
    (Sel::EdgeLabel, "label position_t=0.5"),
    (Sel::EdgeLabel, "label position_t=abc"),
    (Sel::EdgeLabel, "label perpendicular=2"),
    (Sel::EdgeLabel, "label bogus=1"),
    (Sel::EdgeLabel, "label nope"),
    // mode
    (Sel::Node, "mode"),
    (Sel::Node, "mode default"),
    (Sel::Node, "mode resize"),
    (Sel::Node, "mode node-edit"),
    (Sel::Node, "mode nope"),
    // mutation
    (Sel::Node, "mutation"),
    (Sel::Node, "mutation list"),
    (Sel::Node, "mutation list --all"),
    (Sel::Node, "mutation apply"),
    (Sel::Node, "mutation apply no-such-mutation"),
    (Sel::Node, "mutation help"),
    (Sel::Node, "mutation help no-such-mutation"),
    (Sel::Node, "mutation inspect"),
    (Sel::Node, "mutation inspect no-such-mutation"),
    (Sel::Node, "mutation nope"),
    // node
    (Sel::Node, "node"),
    (Sel::Node, "node fit"),
    (Sel::Node, "node edit"),
    (Sel::Node, "node resize"),
    (Sel::Node, "node resize 120 60"),
    (Sel::Node, "node resize a b"),
    (Sel::Node, "node nope"),
    // open / new / save
    (Sel::Node, "open"),
    (Sel::Node, "save"),
    (Sel::Node, "new"),
    // section
    (Sel::TwoSectionNode, "section"),
    (Sel::TwoSectionNode, "section show"),
    (Sel::TwoSectionNode, "section nope"),
    (Sel::TwoSectionNode, "section move"),
    (Sel::Section, "section move dx=1 dy=1"),
    (Sel::Section, "section move dx=1 x=1"),
    (Sel::Section, "section move x=1 y=1"),
    (Sel::Section, "section move dx=abc"),
    (Sel::Section, "section move bogus=1"),
    (Sel::Section, "section resize fill"),
    (Sel::Section, "section resize w=40 h=20"),
    (Sel::Section, "section resize"),
    (Sel::Section, "section resize bogus=1"),
    (Sel::Section, "section text hi"),
    (Sel::Section, "section text hi runs=clear"),
    (Sel::Section, "section text hi runs=nope"),
    (Sel::Section, "section text bogus=1"),
    (Sel::TwoSectionNode, "section add"),
    (Sel::TwoSectionNode, "section add at=0"),
    (Sel::TwoSectionNode, "section add at=abc"),
    (Sel::TwoSectionNode, "section add bogus=1"),
    (Sel::Section, "section delete"),
    (Sel::Section, "section delete bogus=1"),
    (Sel::Section, "section split"),
    (Sel::Section, "section split at=1"),
    (Sel::Section, "section split at=abc"),
    (Sel::Section, "section split bogus=1"),
    (Sel::TwoSectionNode, "section show bogus=1"),
    (Sel::TwoSectionNode, "section show section=99"),
    (Sel::TwoSectionNode, "section show section=abc"),
    (Sel::Section, "section edit"),
    (Sel::Section, "section edit bogus=1"),
    (Sel::Section, "section frame"),
    (Sel::Section, "section frame show"),
    (Sel::Section, "section frame reset"),
    (Sel::Section, "section frame nope"),
    (Sel::Section, "section frame nope palette=coral"),
    (Sel::Section, "section frame palette=My Palette"),
    (Sel::Section, "section frame preset=heavy"),
    (Sel::Section, "section frame bogus=1"),
    (Sel::Section, "section frame preview preset=heavy"),
    (Sel::Section, "section frame preview commit"),
    (Sel::Section, "section frame preview cancel"),
    (Sel::Section, "section frame preview nope"),
    (Sel::Section, "section frame preview NOPE"),
    (Sel::TwoSectionNode, "section frame section=1 preset=heavy"),
    (Sel::TwoSectionNode, "section frame section=abc preset=heavy"),
    (Sel::Node, "section show"),
    (Sel::None, "section show"),
    (Sel::Multi, "section show"),
    (Sel::Multi, "border on"),
    (Sel::Multi, "font size=14"),
    (Sel::Multi, "font min=8"),
    (Sel::Multi, "color bg=#112233"),
    // zoom
    (Sel::Node, "zoom"),
    (Sel::Node, "zoom clear"),
    (Sel::Node, "zoom min=0.5"),
    (Sel::Node, "zoom min=unset"),
    (Sel::Node, "zoom min=abc"),
    (Sel::Node, "zoom min=0"),
    (Sel::Node, "zoom min=2 max=1"),
    (Sel::Node, "zoom bogus=1"),
    (Sel::Node, "zoom nope"),
    (Sel::None, "zoom min=0.5"),
    // Casing. Command names, aliases and positional subverbs are
    // matched case-insensitively console-wide
    // (`commands/mod.rs` § Casing); every subverb popup already
    // filtered its partial that way and inserted the canonical
    // spelling, but only five verbs honored it on the execute
    // side, so a word the popup listed could still be refused
    // when typed out. One row per verb whose dispatch normalizes,
    // each the upper-case twin of a row above.
    (Sel::Node, "help ALL"),
    (Sel::Node, "help BORDER"),
    (Sel::Node, "border PRESET heavy"),
    (Sel::Node, "border PREVIEW COMMIT"),
    (Sel::Node, "canvas BORDER PRESET heavy"),
    (Sel::Node, "canvas SECTION-FRAME FOCUSED show"),
    (Sel::Node, "color PICKER ON"),
    (Sel::Node, "font SET NoSuchFamilyHere"),
    (Sel::Node, "fps ON"),
    (Sel::EdgeLabel, "label CLEAR"),
    (Sel::Node, "mode DEFAULT"),
    (Sel::Node, "mutation LIST"),
    (Sel::Node, "node FIT"),
    (Sel::TwoSectionNode, "section FRAME SHOW"),
    (Sel::Section, "section frame PREVIEW commit"),
    (Sel::Node, "zoom CLEAR"),
    // …and the two halves that stay exact. A kv key is a field
    // name, not a word the user picks, so `TOP=` is unknown on
    // both sides; a palette name is stored verbatim, so the value
    // is compared as written even though the popup finds it
    // case-insensitively.
    (Sel::Node, "border TOP=x"),
    (Sel::Node, "font SIZE=14"),
];

/// `(selection fixture, input line)` — the completion popup for
/// each line (cursor at end of line) is compared against the
/// pinned signature.
pub const COMPLETION_CORPUS: &[(Sel, &str)] = &[
    (Sel::Node, ""),
    (Sel::Node, "co"),
    (Sel::Edge, "b"),
    (Sel::Node, "help "),
    (Sel::Node, "help co"),
    (Sel::Node, "help a"),
    (Sel::Edge, "anchor "),
    (Sel::Edge, "anchor fr"),
    (Sel::Edge, "anchor from="),
    (Sel::Edge, "anchor from=t"),
    (Sel::Edge, "body "),
    (Sel::Edge, "body glyph="),
    (Sel::Edge, "body glyph=d"),
    (Sel::Node, "border "),
    (Sel::Node, "border pre"),
    (Sel::Node, "border preset "),
    (Sel::Node, "border preset h"),
    (Sel::Node, "border color "),
    (Sel::Node, "border palette "),
    (Sel::Node, "border side "),
    (Sel::Node, "border side top "),
    (Sel::Node, "border corner "),
    (Sel::Node, "border corner tl "),
    (Sel::Node, "border show "),
    (Sel::Node, "border show side="),
    (Sel::Node, "border preview "),
    // A kv ahead of the subverb slot puts the line in kv form, so
    // the positional vocabulary is not what would run.
    (Sel::Node, "border color=#fff preset "),
    // …and the slot that *emits* that vocabulary. The gate reached
    // the lookahead above eight lines before it reached this row's
    // arm, so the popup went on offering thirteen subverbs at a
    // slot the verb refuses seven of.
    (Sel::Node, "border color=#fff "),
    // Subverb names are matched case-insensitively on the execute
    // side; the popup follows so the two agree on the same line.
    (Sel::Node, "border PRESET "),
    (Sel::Node, "border preset="),
    (Sel::Node, "border field="),
    (Sel::Node, "border size="),
    (Sel::Node, "border padding "),
    (Sel::Node, "border top="),
    (Sel::Node, "canvas "),
    (Sel::Node, "canvas bo"),
    (Sel::Node, "canvas border "),
    (Sel::Node, "canvas border pre"),
    (Sel::Node, "canvas border preset "),
    (Sel::Node, "canvas border preview "),
    (Sel::Node, "canvas border preset="),
    (Sel::Node, "canvas border show "),
    (Sel::Node, "canvas border color "),
    (Sel::Node, "canvas border palette "),
    (Sel::Node, "canvas border padding "),
    // The depth-2 slots: the value *after* a `side` / `corner`
    // which-arg, on all three canvas surfaces. `EXEC_CORPUS` has
    // pinned `canvas border side top reset` as working since the
    // oracle landed while this table had no counterpart, and that
    // asymmetry is what hid a popup answering with kv keys.
    (Sel::Node, "canvas border side "),
    (Sel::Node, "canvas border side top "),
    (Sel::Node, "canvas border corner "),
    (Sel::Node, "canvas border corner tl "),
    (Sel::Node, "canvas border color=#fff side "),
    (Sel::Node, "canvas Border SIDE top "),
    // The three canvas slots that emit the subverb vocabulary: the
    // `border` subject's, the `section-frame` subject's, and the
    // one past the `focused` modifier. All three offered every
    // positional subverb at a slot `execute_canvas` reads as kv
    // form and rejects seven of.
    (Sel::Node, "canvas color=#fff border "),
    (Sel::Node, "canvas color=#fff section-frame "),
    (Sel::Node, "canvas section-frame color=#fff focused "),
    (Sel::Node, "canvas section-frame "),
    (Sel::Node, "canvas section-frame focused "),
    (Sel::Node, "canvas section-frame focused preview "),
    (Sel::Node, "canvas section-frame preset "),
    (Sel::Node, "canvas section-frame side "),
    (Sel::Node, "canvas section-frame side top "),
    (Sel::Node, "canvas section-frame focused preset "),
    (Sel::Node, "canvas section-frame focused side "),
    (Sel::Node, "canvas section-frame focused side top "),
    (Sel::Node, "canvas section-frame focused corner tl "),
    (Sel::Edge, "cap "),
    (Sel::Edge, "cap from="),
    (Sel::Node, "color "),
    (Sel::Node, "color bg="),
    (Sel::Node, "color picker "),
    (Sel::Node, "color se"),
    (Sel::Node, "color ra"),
    (Sel::TwoSectionNode, "color range="),
    (Sel::Edge, "edge "),
    (Sel::Edge, "edge type="),
    (Sel::Edge, "edge reset="),
    (Sel::Edge, "edge display_mode="),
    (Sel::Node, "font "),
    (Sel::Node, "font se"),
    (Sel::Node, "font size="),
    (Sel::TwoSectionNode, "font section="),
    (Sel::Node, "font ra"),
    (Sel::TwoSectionNode, "font range="),
    (Sel::TwoSectionNode, "color section="),
    (Sel::Node, "fps "),
    (Sel::Node, "fps o"),
    (Sel::Edge, "spacing "),
    (Sel::Edge, "spacing value="),
    (Sel::EdgeLabel, "label "),
    (Sel::EdgeLabel, "label position="),
    (Sel::EdgeLabel, "label text="),
    (Sel::Node, "mode "),
    (Sel::Node, "mode d"),
    (Sel::Node, "mutation "),
    (Sel::Node, "mutation list "),
    (Sel::Node, "mutation apply "),
    (Sel::Node, "node "),
    (Sel::Node, "node r"),
    (Sel::Node, "open "),
    (Sel::Node, "save "),
    (Sel::Node, "new "),
    (Sel::TwoSectionNode, "section "),
    (Sel::TwoSectionNode, "section m"),
    (Sel::TwoSectionNode, "section move "),
    (Sel::TwoSectionNode, "section resize "),
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
    // Casing, popup side. Each row is the upper-case twin of a
    // lower-case row above and must answer identically: the
    // popup's filter and the verb's dispatch read the same way
    // (`commands/mod.rs` § Casing). `border PRE` and `section
    // frame PREVIEW ` answered nothing at all before — a hard
    // stop on a line each verb accepts.
    (Sel::Node, "border PRE"),
    (Sel::Node, "border preset=H"),
    (Sel::Node, "border palette=CO"),
    (Sel::Node, "border side top RE"),
    (Sel::Node, "border show VER"),
    (Sel::Node, "border preview CO"),
    (Sel::Node, "canvas SECTION-FRAME FO"),
    (Sel::Node, "color PICKER "),
    (Sel::Node, "font SE"),
    (Sel::Node, "fps O"),
    (Sel::EdgeLabel, "label CL"),
    (Sel::Node, "mode D"),
    (Sel::Node, "mutation LI"),
    (Sel::Node, "node R"),
    (Sel::TwoSectionNode, "section FRAME "),
    (Sel::TwoSectionNode, "section frame PREVIEW "),
    (Sel::Node, "help AL"),
    (Sel::Node, "zoom C"),
    // The exact half: a kv key filters case-sensitively because
    // `stage_kv` matches it that way, so an upper-case key offers
    // nothing rather than offering a row the verb would reject.
    (Sel::Node, "border TOP"),
];

/// The one outcome whose tail this machine chose rather than the
/// console — pinned as a prefix, with the host-dependent bytes
/// left off the end.
///
/// Rust's `Display` for `io::Error` goes through `strerror_r`, so
/// the words after the path are glibc's and are
/// `LC_MESSAGES`-sensitive: the same `open` of the same missing
/// file reads "No such file or directory" under `LC_ALL=C` and
/// something else under a translated locale. Pinning through the
/// errno string would assert the tester's locale, not the
/// console's behavior. The prefix that stops short of it is not
/// vacuous — it is every byte the console itself contributes, so
/// it pins `open.rs`'s `"open {path}: {e}"` wrapper and the
/// loader's own `"Failed to read file {path}: "` inside it, and a
/// change to either fails this row.
///
/// **Prefix matching is the weaker check, so exactly one row gets
/// it.** The three `border show` readouts sat here too, under a
/// "measured layout" heading claiming their bodies depend on the
/// host's installed fonts. They do not: the cluster count is
/// `node.size.x / (14.0 * BORDER_APPROX_CHAR_WIDTH_FRAC)` — a
/// constant fraction of a constant floor, never a font metric —
/// and the side runs are `SidePattern::render`'s string
/// repetition. Nothing there is measured off a face, so all three
/// are pinned whole in [`EXEC_CORPUS`] instead. Under a prefix
/// they were not merely weaker but *silently* weaker: inserting a
/// whole new line into the readout between `padding:` and `size:`
/// passed every test in the workspace.
pub const EXEC_PREFIX_CORPUS: &[(Sel, &str)] = &[(Sel::Node, "open /nonexistent-dir-xyz/x.mindmap.json")];
