// SPDX-License-Identifier: MPL-2.0

//! The test helper that turns a taught command line into the words a
//! shell would hand the program. Its own module rather than a part of
//! `testing` because it touches no socket: `help`'s tests read it on
//! every platform, and `testing`'s fake server is Unix-only.

/// A minimal shell word splitter: whitespace separates, single and
/// double quotes group, nothing else is special — enough to read what
/// a copy-paste into `sh` would hand the program.
pub fn split(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}
