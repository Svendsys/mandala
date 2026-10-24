// SPDX-License-Identifier: MPL-2.0

//! The family registrar index: one module per command family, one
//! line per family in [`FAMILIES`] (`format/ipc.md` §Command
//! families). A new family adds its module here and its row below,
//! and touches nothing else that another family's PR touches.

mod act;
mod capture;
mod clock;
mod events;
mod input;
mod meta;
mod query;
mod refusal;
mod scene;
mod sidecar;

use super::registry::Family;

/// Every registered family. `describe` and dispatch both walk this
/// table and nothing else.
pub(super) const FAMILIES: &[Family] = &[
    meta::FAMILY,
    act::FAMILY,
    query::FAMILY,
    scene::FAMILY,
    input::FAMILY,
    capture::FAMILY,
    clock::FAMILY,
    events::FAMILY,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ipc::registry::BOOTSTRAP_FAMILY;
    use baumhard::util::doc_fixtures::{documented_paragraph, repo_path, section_text};

    /// The family-map row for `family`: the command names its last
    /// column lists.
    fn documented_commands(family: &str) -> Vec<String> {
        let map = documented_paragraph(
            &repo_path("format/ipc.md"),
            "## Command families",
            "| family | module",
        );
        let label = if family == BOOTSTRAP_FAMILY {
            "*(bootstrap)*".to_string()
        } else {
            format!("`{family}`")
        };
        let row = map
            .lines()
            .find(|line| line.starts_with(&format!("| {label} |")))
            .unwrap_or_else(|| panic!("format/ipc.md's family map has no row for {label}"));
        row.rsplit('|')
            .nth(1)
            .expect("the row ends with its command column")
            .split(',')
            .map(|name| name.trim().trim_matches('`').to_string())
            .collect()
    }

    /// **`describe` stays byte-honest against `format/ipc.md`.** For
    /// every family this build registers, the family map lists
    /// exactly the registered commands, and the row names the module
    /// the family lives in. A family whose issue has not merged is
    /// forward contract in the document and is not checked here,
    /// because `describe` reflects only what the binary registers.
    #[test]
    fn test_every_registered_family_matches_its_family_map_row() {
        for family in FAMILIES {
            let registered: Vec<&str> = family.commands.iter().map(|c| c.name).collect();
            assert_eq!(
                registered,
                documented_commands(family.name),
                "family `{}`: the registry and format/ipc.md's family map disagree",
                family.name
            );
        }
    }

    /// Each registered command has its reference in the document: a
    /// section of its own for the bootstrap pair, a bullet in its
    /// family's section otherwise. Every parameter and result field
    /// it publishes is named there, so `describe` cannot grow a
    /// parameter the document does not mention.
    #[test]
    fn test_every_registered_command_and_field_is_documented() {
        let doc = repo_path("format/ipc.md");
        for family in FAMILIES {
            for command in family.commands {
                let reference = if family.name == BOOTSTRAP_FAMILY {
                    section_text(&doc, &format!("### `{}`", command.name))
                } else {
                    let heading = std::fs::read_to_string(&doc)
                        .expect("format/ipc.md is readable")
                        .lines()
                        .find(|line| line.starts_with(&format!("## `{}` — ", family.name)))
                        .unwrap_or_else(|| panic!("format/ipc.md has no `## `{}` — …` section", family.name))
                        .to_string();
                    section_text(&doc, &heading)
                };
                // A bootstrap command's own heading is its reference,
                // and `section_text` has already refused a missing one.
                assert!(
                    family.name == BOOTSTRAP_FAMILY || reference.contains(&format!("`{}`", command.name)),
                    "format/ipc.md does not reference `{}` in its family's section",
                    command.name
                );
                let fields = command.params.iter().map(|p| p.name);
                for field in fields.chain(command.result.iter().map(|r| r.name)) {
                    assert!(
                        reference.contains(&format!("`{field}`")),
                        "format/ipc.md's reference for `{}` does not name `{field}`",
                        command.name
                    );
                }
            }
        }
    }

    /// **`mandalactl` stays honest against the registry.** The client's
    /// verb table (`crates/mandalactl/src/verbs.rs`) and this registry
    /// name the same commands, so a command that lands without its verb
    /// fails the PR that adds it — and `--help` can never list one the
    /// app does not serve.
    #[test]
    fn test_mandalactl_has_a_verb_for_every_registered_command_and_no_other() {
        use std::collections::BTreeSet;

        let registered: BTreeSet<&str> = FAMILIES
            .iter()
            .flat_map(|family| family.commands.iter().map(|c| c.name))
            .collect();
        let verbs: BTreeSet<&str> = mandalactl::verbs::VERBS.iter().map(|v| v.name).collect();
        let unclaimed: Vec<_> = registered.difference(&verbs).collect();
        let stale: Vec<_> = verbs.difference(&registered).collect();
        assert!(
            unclaimed.is_empty(),
            "registered with no `mandalactl` verb: {unclaimed:?}. Add a `Verb` to crates/mandalactl/src/verbs.rs \
             (summary, parameters as `describe` reports them, an example)"
        );
        assert!(
            stale.is_empty(),
            "`mandalactl` verbs for commands this registry lacks: {stale:?}. Remove them from \
             crates/mandalactl/src/verbs.rs"
        );
    }

    /// Each verb's parameters are the registered command's — name,
    /// type word and required flag, in registry order — because
    /// `mandalactl` types `name=value` arguments from them.
    #[test]
    fn test_every_mandalactl_verb_declares_the_parameters_its_command_registers() {
        for verb in mandalactl::verbs::VERBS {
            let (_, command) = crate::application::ipc::registry::lookup(verb.name)
                .unwrap_or_else(|| panic!("`mandalactl` has a verb for unregistered `{}`", verb.name));
            let registered: Vec<(String, String, bool)> = command
                .params
                .iter()
                .map(|p| (p.name.to_string(), p.ty.word(), p.required))
                .collect();
            let declared: Vec<(String, String, bool)> = verb
                .params
                .iter()
                .map(|p| (p.name.to_string(), mandalactl::verbs::type_word(p.ty), p.required))
                .collect();
            assert_eq!(
                declared, registered,
                "`{}`: crates/mandalactl/src/verbs.rs disagrees with the registry's parameters",
                verb.name
            );
        }
    }

    /// The client refuses a server whose greeting names another
    /// protocol, so its constant must be the server's.
    #[test]
    fn test_mandalactl_speaks_the_protocol_this_server_greets_with() {
        assert_eq!(
            mandalactl::PROTOCOL,
            u64::from(crate::application::ipc::PROTOCOL_VERSION)
        );
    }

    /// The client refuses a request longer than the server reads and
    /// refuses to buffer a reply longer than the server writes, so its
    /// two caps must be the server's.
    #[test]
    fn test_mandalactl_holds_the_same_line_caps_as_the_server() {
        use crate::application::ipc::envelope::{INBOUND_LINE_CAP, OUTBOUND_FRAME_CAP};
        assert_eq!(mandalactl::REQUEST_CAP, INBOUND_LINE_CAP);
        assert_eq!(mandalactl::FRAME_CAP, OUTBOUND_FRAME_CAP);
    }

    /// The convention `ParamSpec::doc` states, held across the *whole*
    /// registry and not only the commands `mandalactl` has a verb for:
    /// a parameter named like a file location documents itself as an
    /// "Absolute path", because `mandalactl` types a command it has no
    /// verb for from `describe` and reads that opening to make a
    /// relative argument absolute. Without it the shell user's
    /// `mandalactl future.export out.json` is refused by the app.
    #[test]
    fn test_every_path_named_parameter_documents_itself_as_an_absolute_path() {
        fn names_a_location(name: &str) -> bool {
            matches!(name, "path" | "dir" | "to_file")
                || ["_path", "_file", "_dir"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }
        for family in FAMILIES {
            for command in family.commands {
                for param in command.params.iter().filter(|p| names_a_location(p.name)) {
                    assert!(
                        param.doc.starts_with("Absolute path"),
                        "`{}` parameter `{}` is named like a file location, so its doc must start with \
                         \"Absolute path\" (see `ParamSpec::doc`): `mandalactl` uses that to make \
                         relative arguments absolute. Got: {:?}",
                        command.name,
                        param.name,
                        param.doc
                    );
                }
            }
        }
    }

    /// `mandalactl` makes a relative path argument absolute, so exactly
    /// the parameters the registry documents as an absolute path are
    /// flagged as paths in its table — no more (a text argument that
    /// happens to look like a path must stay itself) and no fewer (the
    /// shell user's `capture.screenshot after.png` must not be refused).
    #[test]
    fn test_mandalactl_flags_exactly_the_parameters_the_registry_documents_as_absolute_paths() {
        for verb in mandalactl::verbs::VERBS {
            let (_, command) = crate::application::ipc::registry::lookup(verb.name).expect("registered");
            for param in command.params {
                let documented = param.doc.starts_with("Absolute path");
                let flagged = verb.params.iter().any(|p| p.name == param.name && p.path);
                assert_eq!(
                    flagged, documented,
                    "`{}` parameter `{}`: documented as an absolute path = {documented}, flagged in \
                     crates/mandalactl/src/verbs.rs = {flagged}",
                    verb.name, param.name
                );
            }
        }
    }

    /// A verb that prints a result field as bare lines names a field its
    /// command really returns; `Describe` rendering belongs to
    /// `describe` alone.
    #[test]
    fn test_mandalactl_renders_only_fields_its_command_returns() {
        use mandalactl::verbs::Render;
        for verb in mandalactl::verbs::VERBS {
            let (_, command) = crate::application::ipc::registry::lookup(verb.name).expect("registered");
            match verb.render {
                Render::Flat => {}
                Render::Lines(key) => assert!(
                    command.result.iter().any(|field| field.name == key),
                    "`{}` prints `{key}` as lines, which the command does not return",
                    verb.name
                ),
                Render::Describe => assert_eq!(verb.name, "describe"),
            }
        }
    }
}
