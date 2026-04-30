// SPDX-License-Identifier: MPL-2.0

//! Custom derive macros for the Mandala application crate. Today
//! provides exactly one derive: [`ActionClassify`].
//!
//! ## `#[derive(ActionClassify)]`
//!
//! Generates the three classifier methods that previously lived as
//! 113-arm hand-written matches in `keybinds/action/{destructive,
//! context, wasm}.rs`:
//!
//! - `is_destructive(self) -> bool`
//! - `context(self) -> InputContext`
//! - `wasm_compatibility(self) -> WasmCompatibility`
//!
//! The methods are emitted on the discriminant enum (named via
//! strum's `#[strum_discriminants(name(ActionKind))]` — defaults to
//! `ActionKind`) so the generated matches don't have to destructure
//! payloads.
//!
//! ## Per-variant attribute syntax
//!
//! ```ignore
//! #[derive(EnumDiscriminants, ActionClassify)]
//! pub enum Action {
//!     #[action(context = Document, wasm = Compatible)]
//!     Undo,
//!
//!     #[action(context = Document, wasm = Compatible, destructive)]
//!     DeleteSelection,
//!
//!     #[action(context = Document, wasm = NativeOnly, destructive)]
//!     OpenDocument(String),
//!
//!     #[action(context = Console, wasm = NativeOnly)]
//!     ConsoleSubmit,
//! }
//! ```
//!
//! - `context = <ident>` — required. Becomes `InputContext::<ident>`.
//! - `wasm = <ident>` — required. Becomes `WasmCompatibility::<ident>`.
//! - `destructive` — bare flag. Absent ⇒ `false`.
//!
//! ## Forcing function
//!
//! A variant without `#[action(...)]`, missing `context`, or missing
//! `wasm` triggers a `compile_error!` citing the variant name. Same
//! property the hand-written exhaustive matches enforced — the
//! privilege gate's "you must classify every new variant" rule —
//! delivered via attribute presence rather than match-arm presence.
//!
//! ## Discriminant target name
//!
//! Hardcoded to `ActionKind` to match the existing
//! `#[strum_discriminants(name(ActionKind))]` declaration. If the
//! discriminant ever gets renamed, update [`DISCRIMINANT_NAME`]
//! below.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Ident, Variant};

/// Name of the discriminant enum — must match the
/// `#[strum_discriminants(name(...))]` declaration on `Action`.
const DISCRIMINANT_NAME: &str = "ActionKind";

/// Per-variant classification metadata extracted from
/// `#[action(...)]`. All three fields together drive the generated
/// match arm bodies for the three classifier methods.
struct ActionAttrs {
    context: Ident,
    wasm: Ident,
    destructive: bool,
}

/// Derive entry point. Parses each variant's `#[action(...)]`
/// attribute, then emits one `impl ActionKind { ... }` block
/// containing the three classifier methods. Errors during attribute
/// parsing become `compile_error!` invocations targeted at the
/// offending variant, so the build fails with a readable cite
/// rather than an obscure macro panic.
#[proc_macro_derive(ActionClassify, attributes(action))]
pub fn derive_action_classify(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let action_name = &input.ident;

    let Data::Enum(data_enum) = &input.data else {
        return syn::Error::new_spanned(
            &input,
            "ActionClassify can only be derived on enums",
        )
        .to_compile_error()
        .into();
    };

    let discriminant: Ident = syn::parse_str(DISCRIMINANT_NAME).unwrap();

    let mut destructive_arms = Vec::with_capacity(data_enum.variants.len());
    let mut context_arms = Vec::with_capacity(data_enum.variants.len());
    let mut wasm_arms = Vec::with_capacity(data_enum.variants.len());
    let mut errors = Vec::new();

    for variant in &data_enum.variants {
        let variant_name = &variant.ident;
        match parse_action_attrs(variant) {
            Ok(attrs) => {
                let ActionAttrs { context, wasm, destructive } = attrs;
                let destructive_lit = destructive;
                destructive_arms.push(quote! {
                    #discriminant::#variant_name => #destructive_lit
                });
                context_arms.push(quote! {
                    #discriminant::#variant_name => InputContext::#context
                });
                wasm_arms.push(quote! {
                    #discriminant::#variant_name => WasmCompatibility::#wasm
                });
            }
            Err(e) => errors.push(e),
        }
    }

    if !errors.is_empty() {
        let combined = errors.into_iter().reduce(|mut acc, e| {
            acc.combine(e);
            acc
        }).unwrap();
        return combined.to_compile_error().into();
    }

    let expanded: TokenStream2 = quote! {
        impl #discriminant {
            /// Whether the Action this kind represents mutates
            /// persistent state (filesystem, document model
            /// bypassing the undo stack, clipboard) or reaches an
            /// editor modal that mutates on commit. Generated
            /// from the `destructive` flag on each variant's
            /// `#[action(...)]` attribute by `mandala_derive`.
            pub fn is_destructive(self) -> bool {
                match self {
                    #( #destructive_arms ),*
                }
            }

            /// The input context this Action belongs to. Used by
            /// the contextual resolver to filter eligible actions
            /// per modal. Generated from the `context = ...` key
            /// on each variant's `#[action(...)]` attribute.
            pub fn context(self) -> InputContext {
                match self {
                    #( #context_arms ),*
                }
            }

            /// Whether this Action can fire on WASM today.
            /// Generated from the `wasm = ...` key on each
            /// variant's `#[action(...)]` attribute. See
            /// `WASM_CONVERGENCE.md` for the porting path each
            /// `NativeOnly` arm follows.
            pub fn wasm_compatibility(self) -> WasmCompatibility {
                match self {
                    #( #wasm_arms ),*
                }
            }
        }

        // Touch the source enum's name so the macro doesn't emit
        // a warning if the user happens to never name #action_name
        // elsewhere in their crate. (Defensive — usually the user
        // does name it, but this keeps the derive a pure addition.)
        const _: fn() = || {
            let _ = std::mem::size_of::<#action_name>;
        };
    };

    expanded.into()
}

/// Parse a variant's `#[action(...)]` attribute into the typed
/// classification triple. Errors are syn::Errors spanned to the
/// variant's name so `compile_error!` cites the right line.
fn parse_action_attrs(variant: &Variant) -> Result<ActionAttrs, syn::Error> {
    let action_attr = variant
        .attrs
        .iter()
        .find(|a| a.path().is_ident("action"))
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &variant.ident,
                format!(
                    "variant `{}` is missing #[action(context = ..., wasm = ...)] attribute — \
                     ActionClassify requires every variant to declare its classification",
                    variant.ident,
                ),
            )
        })?;

    let mut context: Option<Ident> = None;
    let mut wasm: Option<Ident> = None;
    let mut destructive = false;

    action_attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("context") {
            let value = meta.value()?;
            let ident: Ident = value.parse()?;
            context = Some(ident);
            Ok(())
        } else if meta.path.is_ident("wasm") {
            let value = meta.value()?;
            let ident: Ident = value.parse()?;
            wasm = Some(ident);
            Ok(())
        } else if meta.path.is_ident("destructive") {
            destructive = true;
            Ok(())
        } else {
            Err(meta.error(
                "unknown #[action(...)] key; expected `context = <ident>`, \
                 `wasm = <ident>`, or `destructive`",
            ))
        }
    })?;

    let context = context.ok_or_else(|| {
        syn::Error::new_spanned(
            &variant.ident,
            format!(
                "variant `{}` missing `context = <ident>` in #[action(...)]",
                variant.ident,
            ),
        )
    })?;
    let wasm = wasm.ok_or_else(|| {
        syn::Error::new_spanned(
            &variant.ident,
            format!(
                "variant `{}` missing `wasm = <ident>` in #[action(...)]",
                variant.ident,
            ),
        )
    })?;

    Ok(ActionAttrs { context, wasm, destructive })
}
