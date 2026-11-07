// SPDX-License-Identifier: MPL-2.0

//! The provenance ladder of the macro registry.
//!
//! Macros are looked up by string id and accept definitions from two
//! places: the binary's own bundle and the user's config directory.
//! Those two places form one ladder, and the ladder means two things
//! at once:
//!
//! - **Precedence.** The later tier overrides the earlier one on an id
//!   collision, so `App < User` — the enum derives `Ord` in that order
//!   and the derive is the contract, not a comment.
//! - **Trust.** The ladder is also a trust ranking: `User` content is
//!   the user's own file, the one tier the privileged steps are open
//!   to. The macro dispatcher's privilege gates key off exactly this
//!   distinction; they live next to the macro types in
//!   `crate::application::macros` rather than here, because the
//!   policy is about what a *macro step* may do, not about what a
//!   tier is.
//!
//! A map carries no macros (its verbs replace them, V1-D3 #252), so
//! the two document tiers this ladder once had — the map's and a
//! node's — are gone; content a stranger's map supplies is placed on
//! the ladder deliberately when it returns.

/// Which layer a macro was loaded from.
///
/// Variants are declared in ascending precedence and the derived
/// `Ord` follows that declaration, so `SourceTier::App <
/// SourceTier::User` is a compile-checked way to ask "does this
/// tier lose?". [`SourceTier::ALL`] lists them in the same ascending
/// order.
///
/// `#[non_exhaustive]` because a further tier (a map's verbs, say)
/// has to be placed deliberately in the ladder rather than appended
/// by reflex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum SourceTier {
    /// Shipped with the binary — `assets/macros/application.json`,
    /// embedded with `include_str!`. Lowest precedence, and the only
    /// tier whose content cannot have been modified after the build.
    App,
    /// Loaded from the user's own config file
    /// (`$XDG_CONFIG_HOME/mandala/<name>.json` on native;
    /// `localStorage` on WASM — never a URL query param, which belongs
    /// to whoever composed the link). The user owns the file, so this
    /// is the trusted tier for privileged operations.
    User,
}

impl SourceTier {
    /// Every tier, in ascending precedence. This is the canonical
    /// order: registries index their per-tier slot arrays by
    /// [`SourceTier::index`], which agrees with this list, and
    /// resolution walks it in reverse.
    pub const ALL: [SourceTier; 2] = [SourceTier::App, SourceTier::User];

    /// Number of tiers. Registries size their per-id slot arrays
    /// against this.
    pub const COUNT: usize = Self::ALL.len();

    /// Index into a per-tier slot array. Higher index = higher
    /// precedence, matching [`SourceTier::ALL`]'s order.
    ///
    /// Written as an exhaustive `match` rather than an `as usize`
    /// cast so that adding a variant is a compile error here instead
    /// of a silent renumbering of every stored slot.
    pub(crate) const fn index(self) -> usize {
        match self {
            SourceTier::App => 0,
            SourceTier::User => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user tier outranks the built-in tier — a user's
    /// `macros.json` entry overrides the bundled
    /// one with the same id, never the other way around. This is the
    /// single most user-visible consequence of the ladder.
    #[test]
    fn test_user_tier_outranks_app_tier() {
        assert!(SourceTier::App < SourceTier::User);
        assert!(SourceTier::App.index() < SourceTier::User.index());
    }

    /// The load-bearing pin on the ladder itself: `ALL` is in
    /// ascending precedence, `index()` agrees with it position for
    /// position, and `Ord` agrees with both.
    ///
    /// `Ord` is what precedence comparisons use; `index()` is what
    /// the macro registry's slot array uses. If they ever disagreed,
    /// a macro would resolve from a different tier than the console
    /// reports.
    ///
    /// Note what this pins that a literal `assert_eq!(ALL, [App,
    /// User])` would not: because `index()` is a
    /// hand-written exhaustive `match` and `Ord` is derived from the
    /// *declaration* order, reordering the variants in the `enum`
    /// turns this test red, whereas a literal spelled with the same
    /// variant names would reorder along with it and stay green. The
    /// first loop also proves `ALL` lists both tiers exactly once —
    /// two distinct indices across two slots leaves no room for a
    /// duplicate or an omission.
    #[test]
    fn test_tier_ord_agrees_with_index() {
        for (i, tier) in SourceTier::ALL.iter().enumerate() {
            assert_eq!(tier.index(), i, "{:?} must index at {}", tier, i);
        }
        for pair in SourceTier::ALL.windows(2) {
            assert!(pair[0] < pair[1], "{:?} must rank below {:?}", pair[0], pair[1]);
        }
    }

    /// `COUNT` sizes the registries' slot arrays; a tier missing
    /// from `ALL` would silently shrink them.
    #[test]
    fn test_tier_count_matches_all() {
        assert_eq!(SourceTier::COUNT, 2);
        assert_eq!(SourceTier::COUNT, SourceTier::ALL.len());
    }
}
