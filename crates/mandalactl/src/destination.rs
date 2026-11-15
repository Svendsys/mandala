// SPDX-License-Identifier: MPL-2.0

//! A file this command line names for writing: `record assemble`'s
//! `out=`, and a screenshot's path over `--ws`. Both are written by the
//! workspace's one guarded writer (`guarded_file`); what this adds is
//! the rule for a path a user types (ruling B38.1 §6, as for `maptool
//! export`) and the words a refusal is said in here.
//!
//! **A symlink leads to the file written.** The path is resolved to its
//! real file first (`guarded_file::real_file`): a link that leads
//! nowhere, or to something that is not a file, is refused naming both.
//! The no-clobber rule is then held to that real file, and a write
//! through the link replaces it, leaving the link a link.
//!
//! **A refusal names what was typed** (ruling B38.1, addendum §4).
//! Where the real file is another path, the message says where the
//! typed one leads, in `guarded_file::shown_path`'s form, and the
//! remedy for an existing file is spelled as this tool's option is,
//! `overwrite=true`. The writer's messages are its own, so this is
//! done on their text: the writer publishes the remedy it ends an
//! "exists" refusal with (`guarded_file::OVERWRITE_HINT`), and opens
//! each refusal about a path with that path.

use std::path::{Path, PathBuf};

use guarded_file::PublishRefusal;

/// A path typed on this command line, and the real file it names.
#[derive(Debug)]
pub(crate) struct Destination {
    typed: PathBuf,
    real: PathBuf,
}

impl Destination {
    /// `typed`, resolved to its real file. `Err` when it is a link that
    /// cannot be followed to a file; the message names the link and
    /// where it leads. Cost: [`guarded_file::real_file`]'s.
    pub(crate) fn resolve(typed: &Path) -> Result<Self, String> {
        let real = guarded_file::real_file(typed).map_err(PublishRefusal::message)?;
        Ok(Self {
            typed: typed.to_path_buf(),
            real,
        })
    }

    /// The file a write goes to.
    pub(crate) fn real(&self) -> &Path {
        &self.real
    }

    /// The up-front look at the real file ([`guarded_file::check`]),
    /// said by [`Destination::say`]: for refusing before the work that
    /// produces the file is done.
    pub(crate) fn check(&self, overwrite: bool) -> Result<(), String> {
        guarded_file::check(&self.real, overwrite).map_err(|why| self.say(why))
    }

    /// `why`, said about the path as typed. An existing file's remedy is
    /// `overwrite=true`; where the real file is another path, the
    /// message says where the typed one leads ("`link.gif` leads to
    /// `…/real.gif`, which exists; …"), and shows the real file in
    /// `shown_path`'s form wherever it names it.
    pub(crate) fn say(&self, why: PublishRefusal) -> String {
        let message = why.message();
        let message = match message.strip_suffix(guarded_file::OVERWRITE_HINT) {
            Some(head) => format!("{head}pass overwrite=true to replace it"),
            None => message,
        };
        if self.real == self.typed {
            return message;
        }
        let typed = self.typed.display();
        let shown = guarded_file::shown_path(&self.real);
        let message = message.replace(&self.real.display().to_string(), &shown);
        match message.strip_prefix(&format!("`{shown}` ")) {
            Some(rest) => format!("`{typed}` leads to `{shown}`, which {rest}"),
            None => format!("`{typed}` leads to `{shown}`: {message}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An existing file is refused with this tool's spelling of the
    /// remedy, naming the path typed.
    ///
    /// Control: `say` without the hint rewrite leaves "set `overwrite`".
    #[test]
    fn test_an_existing_file_is_refused_in_this_tools_words() {
        let dir = tempfile::tempdir().expect("a directory");
        let path = dir.path().join("taken.gif");
        std::fs::write(&path, "old").expect("seed");
        let destination = Destination::resolve(&path).expect("a plain path");
        assert_eq!(destination.real(), path, "a plain path is its own real file");
        assert_eq!(
            destination.check(false),
            Err(format!(
                "`{}` exists; pass overwrite=true to replace it",
                path.display()
            ))
        );
        assert_eq!(destination.check(true), Ok(()));
    }

    /// Through a link, the refusal names the link typed and where it
    /// leads, the real file in `shown_path`'s form.
    ///
    /// Control: `say` without the "leads to" arm names only the real
    /// file, which the user never typed.
    #[cfg(unix)]
    #[test]
    fn test_a_refusal_through_a_link_names_the_link_and_where_it_leads() {
        let dir = tempfile::tempdir().expect("a directory");
        let real = dir.path().join("real.gif");
        std::fs::write(&real, "old").expect("seed");
        let link = dir.path().join("link.gif");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let destination = Destination::resolve(&link).expect("a link to a file");
        let real = std::fs::canonicalize(&real).expect("canonical");
        assert_eq!(destination.real(), real, "the write goes to the real file");
        assert_eq!(
            destination.check(false),
            Err(format!(
                "`{}` leads to `{}`, which exists; pass overwrite=true to replace it",
                link.display(),
                real.display()
            ))
        );
        // A refusal that does not open with the real file's name keeps
        // its own words, after where the link leads.
        let other = destination.say(PublishRefusal::Internal("the disk is full".into()));
        assert_eq!(
            other,
            format!(
                "`{}` leads to `{}`: the disk is full",
                link.display(),
                real.display()
            )
        );
    }

    /// A real file that `canonicalize` spells verbatim (Windows'
    /// `\\?\C:\…`) is shown without the prefix, where the message opens
    /// with it and where it names it later. The paths are text here, so
    /// this runs on every host.
    ///
    /// Control: without the replacement of the real file's spelling, the
    /// second message keeps `\\?\`.
    #[test]
    fn test_a_verbatim_real_file_is_shown_as_the_user_knows_it() {
        let destination = Destination {
            typed: PathBuf::from(r"C:\out\link.gif"),
            real: PathBuf::from(r"\\?\C:\kept\real.gif"),
        };
        let exists = PublishRefusal::Invalid(format!(
            r"`\\?\C:\kept\real.gif` exists; {}",
            guarded_file::OVERWRITE_HINT
        ));
        assert_eq!(
            destination.say(exists),
            r"`C:\out\link.gif` leads to `C:\kept\real.gif`, which exists; pass overwrite=true to replace it"
        );
        let failed =
            PublishRefusal::Internal(r"could not write `\\?\C:\kept\real.gif`: the disk is full".into());
        assert_eq!(
            destination.say(failed),
            r"`C:\out\link.gif` leads to `C:\kept\real.gif`: could not write `C:\kept\real.gif`: the disk is full"
        );
    }
}
