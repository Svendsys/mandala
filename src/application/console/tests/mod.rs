// SPDX-License-Identifier: MPL-2.0

//! Integration tests that exercise the console against the canonical
//! `maps/testament.mandala.json` fixture. Split by concern so each file
//! stays focused on one area of the console's surface.
//!
//! - [`fixtures`] — shared `load_test_doc` / `select_first_path` /
//!   `run` helpers.
//! - [`grapheme`] — grapheme-cluster cursor invariants.
//! - [`state`] — `ConsoleState` shape smoke tests.
//! - [`commands`] — per-command execution.
//! - [`applicability`] — per-command `applicable` predicates.
//! - [`completion`] — completion engine.
//! - [`oracle`] — the differential oracle: signatures, the
//!   comparisons, and what the signatures deliberately do not see.
//! - [`revision`] — the corpus held against the document: no console
//!   line changes it, while every editing verb is rebuilt.
//! - [`oracle_corpus`] — the input lines the oracle runs.
//! - [`oracle_expected`] — the answers those lines are pinned to.

mod applicability;
mod commands;
mod completion;
pub(in crate::application::console) mod fixtures;
mod grapheme;
mod oracle;
mod oracle_corpus;
mod oracle_expected;
mod revision;
mod state;
