//! Codex's compressed rollouts (`.jsonl.zst`) and managed-profile sessions
//! in the counter pass. A child of `insights_week::tests`, for its fixture.

use super::*;

const CODEX_ID: &str = "55555555-5555-4555-8555-555555555555";

fn rollout_name() -> String {
    format!("rollout-2026-09-14T09-00-00-{CODEX_ID}.jsonl")
}

/// Compressed as Codex does: one frame that records its size.
fn zst(bytes: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(bytes, 3).unwrap()
}

fn codex_tokens(value: &serde_json::Value) -> serde_json::Value {
    value["rollup"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["source"] == "codex")
        .map(|line| line["tokens"].clone())
        .unwrap_or(serde_json::Value::Null)
}

fn only_row(f: &Fixture) -> CounterRow {
    let store = f.pass.load_cached().unwrap().unwrap();
    assert_eq!(store.rows.len(), 1, "one row");
    store.rows.values().next().unwrap().clone()
}

fn managed(candidate: CounterCandidate) -> CounterCandidate {
    CounterCandidate {
        role: CandidateRole::ManagedProfile,
        ..candidate
    }
}

#[test]
fn a_compressed_rollout_counts_as_its_plain_twin_would() {
    let plain = Fixture::new();
    let path = plain.write(&rollout_name(), &codex_bytes_for(CODEX_ID));
    plain.run(&[candidate(SOURCE_CODEX, &path)]);

    let f = Fixture::new();
    let path = f.write(
        &format!("{}.zst", rollout_name()),
        &zst(&codex_bytes_for(CODEX_ID)),
    );
    let summary = f.run(&[candidate(SOURCE_CODEX, &path)]);
    assert_eq!(summary.read, 1);
    assert_eq!(summary.compressed_unreadable, 0);
    let value = f.week(&[]);
    assert_eq!(codex_tokens(&value), 80 + 30);
    assert_eq!(codex_tokens(&value), codex_tokens(&plain.week(&[])));
    let row = only_row(&f);
    assert!(row.harness_session.is_some(), "its session ID was read");
    assert_eq!(row.not_analyzable, Some(NotAnalyzable::InsightsOnly));
    assert!(
        !f.rows_text().contains(CODEX_ID),
        "no session ID or path stored"
    );
}

/// The compressed file keys as the plain path it replaced, so the row read
/// from the plain file is replaced, never doubled, and a queue entry that
/// names the plain path (#1728's `insights_submitted`) still joins it.
#[test]
fn a_rollout_compressed_after_it_was_counted_replaces_its_row() {
    let f = Fixture::new();
    let plain = f.write(&rollout_name(), &codex_bytes_for(CODEX_ID));
    assert_eq!(f.run(&[candidate(SOURCE_CODEX, &plain)]).read, 1);
    let key = f.pass.keys.load_or_create().unwrap();
    let before: Vec<KeyedDigest> = f
        .pass
        .load_cached()
        .unwrap()
        .unwrap()
        .rows
        .keys()
        .copied()
        .collect();
    assert_eq!(
        before,
        vec![session_key(&key, SOURCE_CODEX, &plain.to_string_lossy())]
    );

    std::fs::remove_file(&plain).unwrap();
    let compressed = f.write(
        &format!("{}.zst", rollout_name()),
        &zst(&codex_bytes_for(CODEX_ID)),
    );
    let summary = f.run(&[candidate(SOURCE_CODEX, &compressed)]);
    assert_eq!(summary.read, 1, "its size changed: read once more");
    let store = f.pass.load_cached().unwrap().unwrap();
    assert_eq!(store.rows.keys().copied().collect::<Vec<_>>(), before);
    assert_eq!(f.week(&[])["sessions_stored"], 1, "replaced, not added");
    assert_eq!(codex_tokens(&f.week(&[])), 80 + 30);
    assert_eq!(
        row_key(&key, SOURCE_CODEX, &compressed),
        session_key(&key, SOURCE_CODEX, &plain.to_string_lossy())
    );
    // Unchanged since: not read again.
    assert_eq!(f.run(&[candidate(SOURCE_CODEX, &compressed)]).read, 0);
}

/// As the watcher hands it: the adapter addresses a compressed rollout by
/// the plain path it replaced, with the size it decompresses to and the
/// last write Codex kept. A row read from the plain file is not read again
/// when Codex compresses it, and a later change reads the compressed file.
#[test]
fn a_compressed_rollout_under_its_address_is_unchanged_and_read_through_the_decoder() {
    let f = Fixture::new();
    let address = f.write(&rollout_name(), &codex_bytes_for(CODEX_ID));
    let plain = candidate(SOURCE_CODEX, &address);
    assert_eq!(f.run(std::slice::from_ref(&plain)).read, 1);
    assert_eq!(only_row(&f).not_analyzable, None);

    std::fs::remove_file(&address).unwrap();
    f.write(
        &format!("{}.zst", rollout_name()),
        &zst(&codex_bytes_for(CODEX_ID)),
    );
    assert_eq!(f.run(std::slice::from_ref(&plain)).read, 0, "unchanged");

    let mut changed = plain.clone();
    changed.modified_at = written() + Duration::minutes(1);
    assert_eq!(f.run(&[changed]).read, 1);
    assert_eq!(
        codex_tokens(&f.week(&[])),
        80 + 30,
        "read through the decoder"
    );
    assert_eq!(
        only_row(&f).not_analyzable,
        Some(NotAnalyzable::InsightsOnly)
    );
}

#[test]
fn only_codex_rollouts_drop_the_compressed_suffix_from_their_key() {
    let key = DigestKey::from_bytes([3; 32]);
    let claude = Path::new("/p/s.jsonl.zst");
    assert_eq!(
        row_key(&key, SOURCE_CLAUDE_CODE, claude),
        session_key(&key, SOURCE_CLAUDE_CODE, "/p/s.jsonl.zst")
    );
    assert_eq!(
        row_key(&key, SOURCE_CODEX, Path::new("/p/rollout-x.jsonl.zst")),
        session_key(&key, SOURCE_CODEX, "/p/rollout-x.jsonl")
    );
    assert_eq!(
        row_key(&key, SOURCE_CODEX, Path::new("/p/rollout-x.jsonl")),
        session_key(&key, SOURCE_CODEX, "/p/rollout-x.jsonl")
    );
}

/// Both files present at once (Codex between writing the compressed file
/// and removing the plain one): one row, read from the plain file.
#[test]
fn a_plain_twin_wins_and_the_session_is_counted_once() {
    let f = Fixture::new();
    let plain = f.write(&rollout_name(), &codex_bytes_for(CODEX_ID));
    // Half written: a compressed twin is never read while the plain exists.
    let mut partial = zst(&codex_bytes_for(CODEX_ID));
    partial.truncate(partial.len() / 2);
    let compressed = f.write(&format!("{}.zst", rollout_name()), &partial);
    let summary = f.run(&[
        candidate(SOURCE_CODEX, &compressed),
        candidate(SOURCE_CODEX, &plain),
    ]);
    assert_eq!(summary.read, 1);
    assert_eq!(summary.compressed_unreadable, 0, "the twin was not opened");
    assert_eq!(f.week(&[])["sessions_stored"], 1);
    assert_eq!(codex_tokens(&f.week(&[])), 80 + 30);
    assert_eq!(only_row(&f).not_analyzable, None, "the plain file's row");
}

#[test]
fn a_corrupt_compressed_rollout_is_unknown_and_counted_never_zero() {
    let f = Fixture::new();
    let mut bytes = zst(&codex_bytes_for(CODEX_ID));
    let cut = bytes.len() / 2;
    bytes.truncate(cut);
    let path = f.write(&format!("{}.zst", rollout_name()), &bytes);
    let summary = f.run(&[candidate(SOURCE_CODEX, &path)]);
    assert_eq!(summary.read, 1);
    assert_eq!(summary.compressed_unreadable, 1);
    assert_eq!(summary.compressed_too_large, 0);
    let row = only_row(&f);
    assert!(matches!(
        row.body,
        RowBody::Unknown {
            reason: UnknownReason::NoUsageCounters,
            ..
        }
    ));
    let value = f.week(&[]);
    assert_eq!(value["sessions_stored"], 1);
    assert_eq!(value["rollup"]["coverage"]["unknown"], 1);
    // Not read again every poll while it stays as it is.
    assert_eq!(f.run(&[candidate(SOURCE_CODEX, &path)]).read, 0);
}

#[test]
fn a_compressed_rollout_past_its_output_bound_stops_and_is_counted() {
    let f = Fixture::new();
    let mut session = codex_bytes_for(CODEX_ID);
    session.push(b'\n');
    // A long run of blank lines: tiny compressed, far over the ratio bound.
    session.resize(
        session.len() + (crate::source::compressed::RATIO_FLOOR_BYTES + 1024 * 1024) as usize,
        b'\n',
    );
    let path = f.write(&format!("{}.zst", rollout_name()), &zst(&session));
    assert!(
        std::fs::metadata(&path).unwrap().len()
            * crate::source::compressed::MAX_DECOMPRESSION_RATIO
            < crate::source::compressed::RATIO_FLOOR_BYTES
    );
    let summary = f.run(&[candidate(SOURCE_CODEX, &path)]);
    assert_eq!(summary.read, 1);
    assert_eq!(summary.compressed_too_large, 1);
    assert_eq!(summary.compressed_unreadable, 0);
    assert!(matches!(only_row(&f).body, RowBody::Unknown { .. }));
}

/// The per-poll budget charges a compressed file by what it decompresses
/// to, as its frame records, not by its size on disk.
#[test]
fn a_compressed_rollout_is_charged_its_decompressed_size() {
    let f = Fixture::new();
    let bytes = codex_bytes_for(CODEX_ID);
    let path = f.write(&format!("{}.zst", rollout_name()), &zst(&bytes));
    assert_eq!(
        read_cost(&candidate(SOURCE_CODEX, &path)),
        bytes.len() as u64
    );
    let plain = f.write("plain.jsonl", &bytes);
    assert_eq!(
        read_cost(&candidate(SOURCE_CODEX, &plain)),
        bytes.len() as u64
    );
}

#[test]
fn a_managed_profile_session_is_counted_and_not_analyzable_here() {
    let f = Fixture::new();
    let path = f.write(&rollout_name(), &codex_bytes_for(CODEX_ID));
    let summary = f.run(&[managed(candidate(SOURCE_CODEX, &path))]);
    assert_eq!(summary.read, 1);
    assert_eq!(codex_tokens(&f.week(&[])), 80 + 30);
    let row = only_row(&f);
    assert!(row.harness_session.is_some());
    assert_eq!(row.not_analyzable, Some(NotAnalyzable::InsightsOnly));
}

#[test]
fn a_managed_profile_session_in_a_never_folder_is_not_read() {
    let f = Fixture::new();
    let path = f.write(&rollout_name(), &codex_bytes_for(CODEX_ID));
    let summary = run_with(
        &f.pass,
        &[managed(candidate(SOURCE_CODEX, &path))],
        &[PROJECT.to_string()],
        true,
    );
    assert_eq!(summary.read, 0);
    assert_eq!(f.week(&[PROJECT.to_string()])["sessions_stored"], 0);
}

/// A managed session copied anywhere else the watcher reads (the
/// post-exit export, or the home folder) is counted once, from the
/// profile, and a row already made from the copy is dropped once the
/// profile's own row exists.
#[test]
fn a_managed_session_is_never_also_counted_from_a_copy() {
    let f = Fixture::new();
    let copies = f.dir.path().join("copies");
    let profile = f.dir.path().join("profile");
    std::fs::create_dir_all(&copies).unwrap();
    std::fs::create_dir_all(&profile).unwrap();
    let copy = copies.join(rollout_name());
    std::fs::write(&copy, codex_bytes_for(CODEX_ID)).unwrap();
    assert_eq!(f.run(&[candidate(SOURCE_CODEX, &copy)]).read, 1);
    assert_eq!(f.week(&[])["sessions_stored"], 1);

    let original = profile.join(rollout_name());
    std::fs::write(&original, codex_bytes_for(CODEX_ID)).unwrap();
    let summary = f.run(&[
        candidate(SOURCE_CODEX, &copy),
        managed(candidate(SOURCE_CODEX, &original)),
    ]);
    assert_eq!(summary.read, 1, "only the profile's");
    assert_eq!(summary.dropped, 1, "the copy's row");
    let value = f.week(&[]);
    assert_eq!(value["sessions_stored"], 1);
    assert_eq!(codex_tokens(&value), 80 + 30);
    let key = f.pass.keys.load_or_create().unwrap();
    let store = f.pass.load_cached().unwrap().unwrap();
    assert!(
        store
            .rows
            .contains_key(&row_key(&key, SOURCE_CODEX, &original))
    );

    // Next poll: still one, nothing read.
    let summary = f.run(&[
        candidate(SOURCE_CODEX, &copy),
        managed(candidate(SOURCE_CODEX, &original)),
    ]);
    assert_eq!(summary.read, 0);
    assert_eq!(f.week(&[])["sessions_stored"], 1);
}

/// The copy keeps its row while the profile's session is still being
/// written: a session is never missing from the count in between.
#[test]
fn a_copy_keeps_its_row_until_the_profile_row_exists() {
    let f = Fixture::new();
    let copies = f.dir.path().join("copies");
    let profile = f.dir.path().join("profile");
    std::fs::create_dir_all(&copies).unwrap();
    std::fs::create_dir_all(&profile).unwrap();
    let copy = copies.join(rollout_name());
    std::fs::write(&copy, codex_bytes_for(CODEX_ID)).unwrap();
    f.run(&[candidate(SOURCE_CODEX, &copy)]);
    let original = profile.join(rollout_name());
    std::fs::write(&original, codex_bytes_for(CODEX_ID)).unwrap();
    let mut writing = managed(candidate(SOURCE_CODEX, &original));
    writing.modified_at = now();
    let summary = f.run(&[candidate(SOURCE_CODEX, &copy), writing]);
    assert_eq!(summary.read, 0);
    assert_eq!(summary.dropped, 0);
    assert_eq!(f.week(&[])["sessions_stored"], 1);
}

/// A managed-profile row, and a compressed one, join their route tally by
/// the session ID the transcript records, as any other row does: where the
/// file lives does not enter the join, so each reads as the route the
/// ledger recorded.
#[test]
fn profile_and_compressed_rows_carry_the_route_their_calls_recorded() {
    let f = Fixture::new();
    let verified_id = CODEX_ID;
    let unverified_id = "66666666-6666-4666-8666-666666666666";
    let profile = f.write(&rollout_name(), &codex_bytes_for(verified_id));
    let compressed = f.write(
        &format!("rollout-2026-09-14T09-30-00-{unverified_id}.jsonl.zst"),
        &zst(&codex_bytes_for(unverified_id)),
    );
    let summary = f.run(&[
        managed(candidate(SOURCE_CODEX, &profile)),
        candidate(SOURCE_CODEX, &compressed),
    ]);
    assert_eq!(summary.read, 2);
    let mut gateway = ledger_call(2, 20, unverified_id);
    gateway.proof = Some(crate::routing::ProofStatus::GatewayOnly);
    let folded = fold_with(
        &f.pass,
        true,
        vec![ledger_call(1, 30, verified_id), gateway],
    )
    .unwrap();
    assert_eq!(folded.folded, 2);
    let value = f.week_routed();
    let mut categories: Vec<String> = sessions_of(&value)
        .iter()
        .map(|row| row["routing"]["category"].as_str().unwrap().to_string())
        .collect();
    categories.sort();
    assert_eq!(
        categories,
        ["routed_unverified", "routed_verified"],
        "{value}"
    );
}
