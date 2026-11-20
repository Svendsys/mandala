// SPDX-License-Identifier: MPL-2.0

//! Opening a span link's address (#252 §3; design §11.2): the one place
//! the app hands a URL to something outside itself.
//!
//! - **Native** hands it to the platform's opener — `xdg-open`, `open`,
//!   `start` — through the `open` crate, detached, so the app never
//!   waits on the browser or the mail client it starts.
//! - **wasm32** opens it in a new tab,
//!   `window.open(url, "_blank", "noopener,noreferrer")` (ruling D3.2
//!   item 1 on #252): the page a map links to gets no handle on
//!   Mandala's tab (`window.opener` is null there), so it cannot
//!   navigate it — which `window.open` without `noopener` would allow,
//!   though an `<a target="_blank">` would not.
//!
//! Only an address a link may lead to is opened: `http`, `https` and
//! `mailto`, and a `mailto` with no `attach` or `attachment` parameter
//! (`check_link_url`; ruling D3.2 item 2). The loader and the writer
//! refuse any other already, so a map cannot hold one; the check here
//! is the last line, at the boundary itself.
//!
//! **The seam tests observe.** Under `cfg(test)` nothing is launched:
//! `open_url` records the address on its thread instead, and
//! `take_opened` hands the tests what was asked for — so the suite,
//! the GPU-backed tests included, never starts a browser (#252 "URL
//! links").

use baumhard::document::values::{check_link_url, LinkRefusal};

/// Open `url` in the platform's handler for it: a browser for `http`
/// and `https`, a mail client for `mailto`. Refused, with nothing
/// launched, for an address of any other scheme or a `mailto` that
/// attaches a file, and when the platform could not start a handler;
/// the message says which.
pub(crate) fn open_url(url: &str) -> Result<(), String> {
    let url = check_link_url(url).map_err(|refusal| match refusal {
        LinkRefusal::Scheme(scheme) => format!(
            "a link opens an `http`, `https` or `mailto` address, and this one is `{}`",
            baumhard::util::log::excerpt(scheme)
        ),
        LinkRefusal::MailAttachment(name) => format!(
            "a `mailto` link may not attach a file, and this one has the parameter `{}`",
            baumhard::util::log::excerpt(name)
        ),
    })?;
    launch(url)
}

/// Hand `url`, checked, to the platform.
#[cfg(all(not(test), not(target_arch = "wasm32")))]
fn launch(url: &str) -> Result<(), String> {
    open::that_detached(url).map_err(|error| format!("could not open the link: {error}"))
}

/// Hand `url`, checked, to the browser: a new tab that has no handle on
/// this one. With `noopener` the call answers `None` rather than the new
/// tab's window, which is not a failure.
#[cfg(all(not(test), target_arch = "wasm32"))]
fn launch(url: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "could not open the link: no window".to_string())?;
    match window.open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer") {
        Ok(_) => Ok(()),
        Err(error) => Err(format!("could not open the link: {error:?}")),
    }
}

#[cfg(test)]
thread_local! {
    /// What [`open_url`] was asked to open on this thread, oldest first.
    static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Record `url` for the tests, instead of launching anything.
#[cfg(test)]
fn launch(url: &str) -> Result<(), String> {
    OPENED.with(|opened| opened.borrow_mut().push(url.to_string()));
    Ok(())
}

/// What [`open_url`] has opened on this thread since the last call,
/// oldest first. Test builds only.
#[cfg(test)]
pub(crate) fn take_opened() -> Vec<String> {
    OPENED.with(|opened| std::mem::take(&mut *opened.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An address of each allowed scheme reaches the seam as it was
    /// given; one of any other scheme is refused before it, naming the
    /// scheme. Control, seen red: with the check removed from
    /// `open_url`, `file:///etc/passwd` reaches the seam.
    #[test]
    fn test_only_an_allowed_scheme_reaches_the_opener() {
        let _ = take_opened();
        for url in [
            "https://example.org/a?b=c",
            "http://example.org",
            "mailto:someone@example.org",
            "HTTPS://example.org",
        ] {
            assert_eq!(open_url(url), Ok(()), "{url}");
        }
        assert_eq!(
            take_opened(),
            [
                "https://example.org/a?b=c",
                "http://example.org",
                "mailto:someone@example.org",
                "HTTPS://example.org"
            ]
        );
        for (url, scheme) in [
            ("file:///etc/passwd", "`file`"),
            ("javascript:alert(1)", "`javascript`"),
            ("example.org", "`example.org`"),
            (" https://example.org", "` https`"),
        ] {
            let refused = open_url(url).expect_err(url);
            assert!(refused.contains(scheme), "{url}: {refused}");
        }
        assert!(take_opened().is_empty(), "nothing refused was opened");
    }

    /// **A `mailto` that attaches a file is not opened** (ruling D3.2
    /// item 2), whatever its case or encoding, the message naming the
    /// parameter; one that attaches nothing is. Control, seen red: with
    /// `check_link_url` not looking at a `mailto`'s parameters, the
    /// first reaches the seam.
    #[test]
    fn test_a_mailto_that_attaches_a_file_does_not_reach_the_opener() {
        let _ = take_opened();
        for (url, name) in [
            ("mailto:a@example.org?attach=/etc/passwd", "`attach`"),
            ("mailto:a@example.org?subject=x&Attachment=y", "`Attachment`"),
            ("mailto:a@example.org?%61ttach=y", "`%61ttach`"),
        ] {
            let refused = open_url(url).expect_err(url);
            assert!(
                refused.contains("may not attach a file") && refused.contains(name),
                "{url}: {refused}"
            );
        }
        assert!(take_opened().is_empty(), "nothing refused was opened");
        assert_eq!(open_url("mailto:a@example.org?subject=attach"), Ok(()));
        assert_eq!(take_opened(), ["mailto:a@example.org?subject=attach"]);
    }

    /// **The browser opens a link with no handle on Mandala's tab**
    /// (ruling D3.2 item 1): its `launch` — compiled for wasm32 alone, so
    /// never linked into this suite, which is why the pin reads the
    /// source (`baumhard::util::rust_source`; TEST_CONVENTIONS §T9) —
    /// makes one `open` call, `window.open` with the target `_blank`
    /// and the features `noopener,noreferrer`.
    ///
    /// Control, seen red: the call as it was,
    /// `open_with_url_and_target(url, "_blank")` (gone).
    #[test]
    fn test_the_browser_opens_a_link_with_no_opener() {
        use baumhard::util::rust_source::{braced_block_after, production_code};
        let code = production_code("src/application/platform/opener.rs");
        let launch = braced_block_after(&code, "#[cfg(all(not(test), target_arch = \"wasm32\"))]")
            .expect("the browser's launch is in the file");
        let call: String = launch.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(call.matches(".open_with_").count(), 1, "one open call: {launch}");
        assert!(
            call.contains(r#".open_with_url_and_target_and_features(url,"_blank","noopener,noreferrer")"#),
            "{launch}"
        );
    }
}
