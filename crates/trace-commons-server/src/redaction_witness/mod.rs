// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The redaction witness: proving a redacted trace artifact derives from the
//! raw bytes a NEAR AI inference receipt covers, by redaction alone.
//!
//! The correspondence check applies a client-supplied redaction span list to
//! the raw text and requires byte equality with the submitted artifact. It
//! never replays a classifier, so classifier nondeterminism cannot fail an
//! honest submission, and there is no fuzzy matcher to smuggle content past.
//!
//! Nothing in this module logs. Raw text, redacted text and span lists are all
//! contributor content -- and a span list's *shape* reveals what the detector
//! found, so it is if anything more sensitive than the text it describes.

pub mod certificate;
pub mod correspondence;
