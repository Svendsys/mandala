// SPDX-License-Identifier: MPL-2.0

//! Opening a span link's address (#252 §3; design §11.2): the one place
//! the app hands a URL to something outside itself.
//!
//! - **Native** hands it to the platform's opener — `xdg-open`, `open`,
//!   `start` — through the `open` crate, detached, so the app never
//!   waits on the browser or the mail client it starts.
//! - **wasm32** opens it in a new tab, `window.open(url, "_blank")`.
//!
//! Only an address a link may lead to is opened: `http`, `https` and
//! `mailto` (`check_link_url`). The loader and the writer refuse any
//! other already, so a map cannot hold one; the check here is the last
//! line, at the boundary itself.
//!
//! **The seam tests observe.** Under `cfg(test)` nothing is launched:
//! `open_url` records the address on its thread instead, and
//! `take_opened` hands the tests what was asked for — so the suite,
//! the GPU-backed tests included, never starts a browser (#252 "URL
//! links").

use baumhard::document::values::check_link_url;

/// Open `url` in the platform's handler for it: a browser for `http`
/// and `https`, a mail client for `mailto`. Refused, with nothing
/// launched, for an address of any other scheme, and when the platform
/// could not start a handler; the message says which.
pub(crate) fn open_url(url: &str) -> Result<(), String> {
    let url = check_link_url(url).map_err(|scheme| {
        format!(
            "a link opens an `http`, `https` or `mailto` address, and this one is `{}`",
            baumhard::util::log::excerpt(scheme)
        )
    })?;
    launch(url)
}

/// Hand `url`, checked, to the platform.
#[cfg(all(not(test), not(target_arch = "wasm32")))]
fn launch(url: &str) -> Result<(), String> {
    open::that_detached(url).map_err(|error| format!("could not open the link: {error}"))
}

/// Hand `url`, checked, to the browser: a new tab.
#[cfg(all(not(test), target_arch = "wasm32"))]
fn launch(url: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or_else(|| "could not open the link: no window".to_string())?;
    match window.open_with_url_and_target(url, "_blank") {
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
}
