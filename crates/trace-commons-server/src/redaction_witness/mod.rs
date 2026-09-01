// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The redaction witness: proving a redacted trace artifact derives from
//! some raw text by redaction alone.
//!
//! The correspondence check applies a client-supplied redaction span list to
//! the raw text and requires byte equality with the submitted artifact. It
//! never replays a classifier, so classifier nondeterminism cannot fail an
//! honest submission, and there is no fuzzy matcher to smuggle content past.
//!
//! **What this module does not establish.** Nothing here ties the raw text to
//! a NEAR AI inference receipt. `chat_id`, `model` and the token counts on a
//! certificate are witness self-report, checked against no receipt, because
//! this module holds none. The certificate carries them so that the witness
//! service -- which does hold a receipt -- can bind them there. Until it
//! does, a verified certificate says the artifact derives from raw text the
//! witness saw, not that anybody paid for that inference.
//!
//! **The artifact must reach the witness byte for byte.** The certificate's
//! digest is over exactly the bytes `check_correspondence` compared, and
//! verification hashes exactly the bytes the server holds. Any re-encoding,
//! wrapper or added trailing newline on either side fails closed.
//!
//! Nothing in this module logs. Raw text, redacted text and span lists are all
//! contributor content -- and a span list's *shape* reveals what the detector
//! found, so it is if anything more sensitive than the text it describes.

pub mod certificate;
pub mod correspondence;
pub mod verification;
