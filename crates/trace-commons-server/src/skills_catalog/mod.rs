// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The public Skills catalog's server side: a crawler over a bounded set of
//! public sources (the reviewed source file, plugin marketplaces one hop
//! deep, awesome-lists, and app suggestions), the checks each skill goes
//! through, the PostgreSQL state it keeps (V160), and the exporter that
//! writes the unsigned payload the owner signs and publishes. Spec:
//! `docs/superpowers/specs/2026-11-15-skills-catalog-server.md`.
//!
//! Nothing here signs or uploads, and nothing searches GitHub at large.
//! Operational logs carry labels and counts only, never a repository name a
//! user reported.

pub mod checks;
pub mod crawl;
pub mod export;
pub mod fetch;
pub mod sources;
pub mod store;
