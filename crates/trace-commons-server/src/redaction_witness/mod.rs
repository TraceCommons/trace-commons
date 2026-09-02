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
//! a NEAR AI inference receipt, and the certificate no longer pretends to: the
//! inference fields it once carried are gone, because no trace population in
//! this repo can fill them honestly. A verified certificate says the artifact
//! derives from raw text the witness saw, and reports the verdict a known
//! program reached over it. It says nothing about who paid for the inference
//! behind the trace. The reasoning, and why those fields are not coming back
//! as optional ones, is recorded in [`certificate`].
//!
//! **One correspondence check must not mint N certificates.** Nothing here
//! bounds that: an empty span list is legal, so a proof over an artifact
//! costs one call, and `CorrespondenceProof` not being `Clone` buys one
//! certificate per proof and nothing beyond it. Inside the attested enclave
//! that costs nothing today, because the enclave is the only thing holding
//! raw bytes. It becomes load-bearing at the service slice, where the
//! certificate-to-receipt binding lands: N certificates against N receipts
//! for one check is the shape to refuse there, and it has to be refused
//! there, because this module cannot see a receipt.
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
