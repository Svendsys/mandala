// SPDX-License-Identifier: MPL-2.0

//! The missing-font prompt (#329): what the app tells the user when a
//! map names installed font families this machine lacks, and the two
//! ways out it offers — install the font, then rescan; or pick a
//! replacement, which rewrites the map's references to the family as one
//! undoable edit.
//!
//! The report itself is the document's
//! ([`baumhard::document::fonts::missing_families`]), computed when it is
//! asked for and never kept up per edit. This module says it:
//!
//! - **[`on_open`]**, when a map is opened (at startup, and by `open` /
//!   `new`), before its view is projected: on native, one targeted
//!   rescan of the host for the families the map lacks, so a font
//!   installed since launch is drawn without a restart; then the notice.
//! - **The notice** ([`notice`]), on the status line, on both targets,
//!   until the user dismisses it (`ExitMode`, the status line's own
//!   dismissal) or resolves the last missing family.
//! - **The prompt** (`report_lines`, `replace_line`): on native the
//!   console opens with the report and a pre-filled
//!   `font replace "<family>" ` line, its completion offering every
//!   family a map can name. It does not hold the map: the map is drawn
//!   and loaded behind it, and Escape closes it like any console.
//!   The browser has no console shell (CLAUDE.md "Dual-target status"),
//!   so there it is the notice, and the same choice over IPC
//!   (`query.fonts`, `act.action ReplaceFontFamily`).
//! - **The replacement** ([`replace`]), one request through the writer,
//!   which the console's `font replace`, the `ReplaceFontFamily` Action
//!   and IPC share.

use baumhard::document::fonts::{missing_families, MissingFamily, STAND_IN};
use baumhard::document::values::FontRef;
use baumhard::document::writer::ChangeSource;

#[cfg(not(target_arch = "wasm32"))]
use crate::application::console::OutputLine;
use crate::application::document::MindMapDocument;
use crate::application::fields::{self, Refused};

/// What opening `doc` does about the fonts it names, before its view is
/// projected: on native, look the host up again for the installed
/// families it names that the font system lacks
/// (`baumhard::font::fonts::rescan_installed`, native only), so a
/// font installed since this process started is drawn; then set the
/// notice. Returns the families still missing.
///
/// A map that names only compiled families returns at once: no rescan,
/// no font system read. The browser reads no installed font, so there
/// every installed family a map names is missing.
///
/// Cost: [`missing_families`]; when one is missing on native, one host
/// font scan.
pub(crate) fn on_open(doc: &mut MindMapDocument) -> Vec<MissingFamily> {
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut missing = missing_families(doc.doc());
    #[cfg(not(target_arch = "wasm32"))]
    if !missing.is_empty() {
        let found =
            baumhard::font::fonts::rescan_installed(missing.iter().map(|missing| missing.family.as_str()));
        if !found.is_empty() {
            log::info!("fonts: found {found:?} installed since this process started");
            missing.retain(|missing| !found.iter().any(|name| name == missing.family.as_str()));
        }
    }
    if !missing.is_empty() {
        log::warn!(
            "fonts: this map names {} this machine lacks, drawn in {STAND_IN}: {}",
            plural(missing.len(), "font", "fonts"),
            names(&missing)
        );
    }
    doc.set_font_notice(notice(&missing));
    missing
}

/// The status line's notice for `missing`: `None` when nothing is.
/// Short, because it is a status line. O(the names' length).
pub(crate) fn notice(missing: &[MissingFamily]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "{} missing, drawn in {STAND_IN}: {}",
        plural(missing.len(), "font", "fonts"),
        names(missing)
    ))
}

/// The prompt's report: which families are missing, where the map names
/// them, what their text is drawn in, and the two ways out. One line
/// each.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn report_lines(missing: &[MissingFamily]) -> Vec<OutputLine> {
    let mut lines = vec![OutputLine::plain(format!(
        "This map names {} this machine lacks. Its text is drawn in {STAND_IN} until you choose:",
        plural(missing.len(), "font", "fonts")
    ))];
    for family in missing {
        let mut line = format!(
            "  \"{}\" — {}",
            family.family,
            plural(family.elements, "element", "elements")
        );
        if !family.definitions.is_empty() {
            line.push_str(&format!(
                "; also named by {}, which a replacement leaves as it is",
                family.definitions.join(", ")
            ));
        }
        lines.push(OutputLine::plain(line));
    }
    lines.push(OutputLine::plain(
        "Install it, then `font rescan` (or open the map again);",
    ));
    lines.push(OutputLine::plain(
        "or replace it: `font replace \"<family>\" <font>`, one edit that undo reverts. Esc closes this.",
    ));
    lines
}

/// The console line the prompt is opened with: `font replace "<family>" `
/// for the first missing family, so the completion offers the fonts it
/// could be replaced with. `None` when nothing is missing.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn replace_line(missing: &[MissingFamily]) -> Option<String> {
    missing
        .first()
        .map(|family| format!("font replace \"{}\" ", family.family))
}

/// Replace every reference `doc`'s map makes to the installed family
/// `family` with `with`, a family a map can name, as one request through
/// the writer — one undo entry, whose undo restores them all — declared
/// by `source`; then set the notice to what is still missing. `Ok(false)`
/// when the map names `family` nowhere. The one body of the console's
/// `font replace`, the `ReplaceFontFamily` Action and IPC.
///
/// Errors: `family` that is a compiled face's name or no family name, a
/// `with` that cannot be one, and whatever the writer refuses.
///
/// Cost: one walk of the tree for the steps, the request, and
/// [`missing_families`].
pub(crate) fn replace(
    doc: &mut MindMapDocument,
    family: &str,
    with: &str,
    source: ChangeSource,
) -> Result<bool, Refused> {
    let from = match baumhard::document::format::font(family) {
        Ok(FontRef::Installed(from)) => from,
        Ok(FontRef::Compiled(_)) => {
            return Err(Refused::Because(format!(
                "font replace: '{family}' is compiled in, not an installed family"
            )))
        }
        Err(fault) => return Err(Refused::Because(format!("font replace: '{family}': {fault}"))),
    };
    let to = baumhard::document::format::font(with)
        .map_err(|fault| Refused::Because(format!("font replace: '{with}': {fault}")))?;
    let steps =
        baumhard::document::fonts::replace_steps(doc.doc(), &from, &to).map_err(|error| error.to_string());
    let applied = fields::apply(doc, steps, "font replace", source)?;
    let missing = missing_families(doc.doc());
    doc.set_font_notice(notice(&missing));
    Ok(applied)
}

/// Look the host up again for the installed families `doc`'s map names
/// that this machine lacks: what `font rescan` and the `RescanFonts`
/// Action do once the user has installed one. Returns the families found
/// and those still missing, and sets the notice. A caller that found any
/// projects the view again, since every glyph of a found family moves
/// from the stand-in to its own face. Native only: the browser reads no
/// installed font.
///
/// Cost: [`missing_families`]; when one is missing, one host font scan.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rescan(doc: &mut MindMapDocument) -> (Vec<String>, Vec<MissingFamily>) {
    let missing = missing_families(doc.doc());
    let found =
        baumhard::font::fonts::rescan_installed(missing.iter().map(|missing| missing.family.as_str()));
    let still: Vec<MissingFamily> = missing
        .into_iter()
        .filter(|missing| !found.iter().any(|name| name == missing.family.as_str()))
        .collect();
    doc.set_font_notice(notice(&still));
    (found, still)
}

/// `query.fonts`' reply for `doc` (`format/ipc.md` §`query.fonts`):
/// every loaded family and its source, the installed families the map
/// names and whether this machine has each, the names of those it lacks,
/// and the stand-in their text is drawn in. Read-only.
///
/// Cost: the family index (the host's font scan, if nothing has paid
/// for it) and [`installed_families`](baumhard::document::fonts::installed_families).
pub(crate) fn ipc_json(doc: &MindMapDocument) -> serde_json::Value {
    use baumhard::font::fonts::{installed_family_loaded, loaded_families, FamilySource};
    let families: Vec<serde_json::Value> = loaded_families()
        .iter()
        .map(|family| {
            serde_json::json!({
                "name": family.name,
                "source": match family.source {
                    FamilySource::Compiled(_) => "compiled",
                    FamilySource::Installed => "installed",
                },
            })
        })
        .collect();
    let mut missing = Vec::new();
    let named: Vec<serde_json::Value> = baumhard::document::fonts::installed_families(doc.doc())
        .into_iter()
        .map(|named| {
            let installed = installed_family_loaded(named.family.as_str());
            if !installed {
                missing.push(named.family.to_string());
            }
            serde_json::json!({
                "family": named.family.as_str(),
                "installed": installed,
                "elements": named.elements,
                "definitions": named.definitions,
            })
        })
        .collect();
    serde_json::json!({
        "families": families,
        "named": named,
        "missing": missing,
        "stand_in": STAND_IN,
    })
}

/// The missing families' names, quoted and comma-separated.
fn names(missing: &[MissingFamily]) -> String {
    missing
        .iter()
        .map(|missing| format!("\"{}\"", missing.family))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `n` and the noun for one or for many: `1 font`, `2 fonts`.
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::{parse, Args, ParseResult};
    use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ExecResult};

    /// A map naming the installed family `Nope Sans` on a node and a
    /// span, `Other Missing` on a span, and the compiled `Public Pixel`
    /// on another. Neither installed family is installed anywhere this
    /// runs.
    const MAP: &str = r#"{"format":1,"name":"f","nodes":[{"id":1,"position":[0.0,0.0],"font":"Nope Sans","sections":[{"spans":[{"text":"a","font":"Nope Sans"},{"text":"b","font":"Other Missing"},{"text":"c","font":"Public Pixel"}]}]}]}"#;

    fn doc() -> MindMapDocument {
        MindMapDocument::from_json_str(MAP, None, 50).expect("the map loads")
    }

    /// Run the console line `line` on `doc`: its message, or its refusal,
    /// and the side effect it asked for.
    fn line(doc: &mut MindMapDocument, line: &str) -> (Result<String, String>, Option<ConsoleSideEffect>) {
        let ParseResult::Ok { cmd, args } = parse(line) else {
            panic!("{line:?} does not parse");
        };
        let mut effects = ConsoleEffects::new(doc);
        let result = match (cmd.execute)(&Args::new(&args), &mut effects) {
            ExecResult::Ok(message) => Ok(message),
            ExecResult::Lines(lines) => Ok(lines.into_iter().map(|l| l.text).collect::<Vec<_>>().join("\n")),
            ExecResult::Err(message) => Err(message),
        };
        (result, effects.side_effect.take())
    }

    /// **Opening a map that names fonts this machine lacks says so**: the
    /// notice names both families, by name, sorted, with the stand-in;
    /// a map that names only compiled families gets none, and the
    /// status line's dismissal clears it. Control: the compiled map,
    /// through the same `on_open`, leaves no notice.
    #[test]
    fn test_opening_a_map_with_missing_fonts_sets_the_notice() {
        let mut doc = doc();
        let missing = on_open(&mut doc);
        assert_eq!(
            missing.iter().map(|m| m.family.to_string()).collect::<Vec<_>>(),
            ["Nope Sans", "Other Missing"]
        );
        assert_eq!(
            doc.font_notice(),
            Some("2 fonts missing, drawn in Inter Variable: \"Nope Sans\", \"Other Missing\"")
        );
        let mut compiled = MindMapDocument::from_json_str(
            r#"{"format":1,"name":"f","nodes":[{"id":1,"position":[0.0,0.0],"font":"Public Pixel"}]}"#,
            None,
            50,
        )
        .expect("the map loads");
        assert!(on_open(&mut compiled).is_empty());
        assert_eq!(compiled.font_notice(), None, "control: nothing is missing");
        doc.dismiss_status();
        assert_eq!(doc.font_notice(), None, "dismissed with the status line");
    }

    /// **The prompt's report says which, where, in what, and how out**:
    /// a heading, a line per family with its count, and the two ways
    /// out; the pre-filled line names the first family.
    #[test]
    fn test_the_prompt_reports_each_missing_family_and_both_ways_out() {
        let missing = baumhard::document::fonts::missing_families(doc().doc());
        let lines: Vec<String> = report_lines(&missing).into_iter().map(|l| l.text).collect();
        assert_eq!(
            lines.len(),
            5,
            "a heading, two families and the two ways out: {lines:?}"
        );
        assert_eq!(
            lines[0],
            "This map names 2 fonts this machine lacks. Its text is drawn in Inter Variable until you choose:"
        );
        assert_eq!(lines[1], "  \"Nope Sans\" — 2 elements");
        assert_eq!(lines[2], "  \"Other Missing\" — 1 element");
        assert!(
            lines[3].contains("font rescan") && lines[4].contains("font replace"),
            "{lines:?}"
        );
        assert_eq!(
            replace_line(&missing).as_deref(),
            Some("font replace \"Nope Sans\" ")
        );
        assert_eq!(replace_line(&[]), None, "control: nothing to prompt for");
    }

    /// **`font replace` is the replacement's body on the console**: it
    /// rewrites every reference as one undo entry and says what is still
    /// missing; it refuses a compiled family as the one to replace, and a
    /// name that cannot be one as the replacement, applying nothing.
    /// Controls: the refusals leave the history where it was.
    #[test]
    fn test_font_replace_rewrites_the_map_and_reports_what_is_still_missing() {
        let mut doc = doc();
        on_open(&mut doc);
        let depth = doc.doc().history().count();
        let (refused, _) = line(&mut doc, "font replace \"Public Pixel\" Norse");
        assert_eq!(
            refused,
            Err("font replace: 'Public Pixel' is compiled in, not an installed family".to_string())
        );
        let (refused, _) = line(&mut doc, "font replace \"Nope Sans\" \"\"");
        assert!(refused.is_err(), "{refused:?}");
        assert_eq!(doc.doc().history().count(), depth, "the refusals applied nothing");
        let (answer, _) = line(&mut doc, "font replace \"Nope Sans\" Norse");
        assert_eq!(
            answer,
            Ok(
                "font replace: \"Nope Sans\" is now Norse; 1 font missing, drawn in Inter Variable: \
                 \"Other Missing\""
                    .to_string()
            )
        );
        assert_eq!(doc.doc().history().count(), depth + 1, "one undo entry");
        let saved = baumhard::document::format::save(doc.doc()).expect("saves");
        assert!(
            !saved.contains("Nope Sans") && saved.matches("\"Norse\"").count() == 2,
            "{saved}"
        );
        let (answer, _) = line(&mut doc, "font replace \"Other Missing\" \"Galatia SIL\"");
        assert_eq!(
            answer,
            Ok("font replace: \"Other Missing\" is now Galatia SIL; no font is missing".to_string())
        );
        assert_eq!(doc.font_notice(), None);
    }

    /// **A row naming a family whose face cannot draw that name is drawn
    /// in the default face** (#329): `Critters1DC`'s character map lacks
    /// most of its own letters, so its completion row and its `font list`
    /// row pin no face, where `Inter Variable`'s pin their own. Control:
    /// `row_face` pinning every family to itself turns the `Critters1DC`
    /// arms red, the row drawn half in `.notdef`-prone symbols.
    #[test]
    fn test_a_family_whose_face_cannot_draw_its_name_is_listed_in_the_default_face() {
        use crate::application::console::commands::font::font_family_completions;
        assert!(
            !baumhard::font::fonts::compiled_family_maps("Critters1DC", "Critters1DC"),
            "precondition: the face lacks letters of its name"
        );
        let rows = font_family_completions("");
        let row = |name: &str| {
            rows.iter()
                .find(|row| row.display == name)
                .unwrap_or_else(|| panic!("{name} is offered"))
                .font_family
                .clone()
        };
        assert_eq!(row("Critters1DC"), None, "drawn in the default face");
        assert_eq!(
            row("Inter Variable").as_deref(),
            Some("Inter Variable"),
            "drawn in its own"
        );
        let ParseResult::Ok { cmd, args } = parse("font list") else {
            panic!("font list parses");
        };
        let mut doc = doc();
        let mut effects = ConsoleEffects::new(&mut doc);
        let ExecResult::Lines(lines) = (cmd.execute)(&Args::new(&args), &mut effects) else {
            panic!("font list lists");
        };
        let listed = |name: &str| {
            lines
                .iter()
                .find(|line| line.text == name)
                .unwrap_or_else(|| panic!("{name} is listed"))
                .font_family
                .clone()
        };
        assert_eq!(listed("Critters1DC"), None);
        assert_eq!(listed("Inter Variable").as_deref(), Some("Inter Variable"));
    }

    /// `font rescan` asks the executor for the rescan, which reports and
    /// projects (`console_input::exec`); the verb itself changes nothing.
    #[test]
    fn test_font_rescan_asks_for_the_rescan() {
        let mut doc = doc();
        let (answer, effect) = line(&mut doc, "font rescan");
        assert_eq!(answer, Ok(String::new()));
        assert!(
            matches!(effect, Some(ConsoleSideEffect::RescanFonts)),
            "{effect:?}"
        );
    }

    /// `font list` marks every installed family `(installed)` and no
    /// compiled one. On a host with no installed family the mark is
    /// untested, and the test says so.
    #[test]
    fn test_font_list_marks_the_installed_families() {
        use baumhard::font::fonts::{loaded_families, FamilySource};
        let mut doc = doc();
        let (listed, _) = line(&mut doc, "font list");
        let listed = listed.expect("font list");
        let families = loaded_families();
        let rows: Vec<&str> = listed.lines().collect();
        assert_eq!(rows.len(), families.len(), "one row per loaded family");
        for (row, family) in rows.iter().zip(families.iter()) {
            let want = match family.source {
                FamilySource::Compiled(_) => family.name.clone(),
                FamilySource::Installed => format!("{}  (installed)", family.name),
            };
            assert_eq!(*row, want);
        }
        if !families
            .iter()
            .any(|family| family.source == FamilySource::Installed)
        {
            baumhard::util::test_skip::note_skipped(
                "missing_fonts: no installed family on this host, so `font list`'s mark is untested",
            );
        }
    }
}
