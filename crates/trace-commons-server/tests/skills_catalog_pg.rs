// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Skills catalog's crawl state and suggestions (V160) on a real
//! PostgreSQL: the runtime may only call the suggestion function, the guard
//! keeps first-seen times and hash history, and the exporter reads it back.
//! One test function, because its steps share rows and the pending cap.

use chrono::{Duration, Utc};
use trace_commons_protocol::skills_catalog::{Environment, SkillSuggestion, SourceKind};
use trace_commons_server::{
    config::DatabaseConfig,
    db::{Database, SkillsCatalogSuggestionOutcome, postgres::PgBackend},
    skills_catalog::{
        crawl::crawl_repository,
        export::{Suppressions, build_document},
        fetch::{FixtureFetcher, FixtureRepository},
        sources::{CrawlTarget, Seed},
        store::SkillsCatalogStore,
    },
};

async fn backends() -> Option<(PgBackend, PgBackend)> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return None;
    };
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .unwrap();
    admin.run_migrations().await.unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url.set_query(Some("options=-c%20role%3Dtrace_ingest_runtime"));
    let runtime = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 2))
        .await
        .unwrap();
    Some((admin, runtime))
}

const HEAD_1: &str = "1111111111111111111111111111111111111111";
const HEAD_2: &str = "2222222222222222222222222222222222222222";

fn skill(name: &str, body: &str) -> Vec<u8> {
    format!("---\nname: {name}\ndescription: Does {name}.\nlicense: MIT\n---\n{body}").into_bytes()
}

fn seed_target(repository: &str) -> CrawlTarget {
    CrawlTarget {
        repository: repository.into(),
        kind: SourceKind::Seed,
        seed: Some(Seed {
            publisher_id: "pgseed".into(),
            publisher_name: "PG Seed".into(),
            instructions_url: "https://example.com/install".into(),
        }),
        roots: vec!["skills".into()],
    }
}

fn suggestion(repository: &str, path: &str) -> SkillSuggestion {
    SkillSuggestion {
        repository: repository.into(),
        path: path.into(),
        revision: HEAD_1.into(),
    }
}

async fn guard_query_count(admin: &PgBackend, sql: &str) -> i64 {
    let mut client = admin.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute(
        "GRANT trace_skills_catalog_guard TO CURRENT_USER;
         SET LOCAL ROLE trace_skills_catalog_guard",
    )
    .await
    .unwrap();
    let count: i64 = tx.query_one(sql, &[]).await.unwrap().get(0);
    tx.commit().await.unwrap();
    count
}

async fn guard_execute(admin: &PgBackend, sql: &str) {
    let mut client = admin.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE trace_skills_catalog_guard")
        .await
        .unwrap();
    tx.batch_execute(sql).await.unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn skills_catalog_state_on_postgres() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    // The migrator holds the guard only during V160; an operator grants it
    // to the crawler login. The test plays that operator.
    admin
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .batch_execute("GRANT trace_skills_catalog_guard TO CURRENT_USER")
        .await
        .unwrap();
    guard_execute(
        &admin,
        "DELETE FROM trace_skills_catalog_suggestions WHERE repository LIKE 'pgtest-%';
         DELETE FROM trace_skills_catalog_repositories WHERE repository LIKE 'pgtest-%';",
    )
    .await;

    // The runtime: one function, nothing else.
    let first = suggestion("pgtest-a/skills", "skills/x/SKILL.md");
    assert_eq!(
        runtime
            .record_skills_catalog_suggestion(&first)
            .await
            .unwrap(),
        SkillsCatalogSuggestionOutcome::Stored
    );
    assert_eq!(
        runtime
            .record_skills_catalog_suggestion(&first)
            .await
            .unwrap(),
        SkillsCatalogSuggestionOutcome::Counted
    );
    let client = runtime.trace_pool_for_test().get().await.unwrap();
    for sql in [
        "SELECT count(*) FROM trace_skills_catalog_suggestions",
        "SELECT count(*) FROM trace_skills_catalog_skills",
        "SELECT count(*) FROM trace_skills_catalog_repositories",
        "DELETE FROM trace_skills_catalog_suggestions",
        "INSERT INTO trace_skills_catalog_suggestions (repository, path, revision)
         VALUES ('pgtest-z/z', 'SKILL.md', '1111111111111111111111111111111111111111')",
    ] {
        assert!(
            client.execute(sql, &[]).await.is_err(),
            "runtime may not: {sql}"
        );
    }
    // The function's own checks back the service's validation.
    assert!(
        client
            .query_one(
                "SELECT trace_skills_catalog_suggest('Bad/Repo', 'SKILL.md', $1)",
                &[&HEAD_1]
            )
            .await
            .is_err()
    );
    assert!(
        client
            .query_one(
                "SELECT trace_skills_catalog_suggest('pgtest-a/b', '../SKILL.md', $1)",
                &[&HEAD_1]
            )
            .await
            .is_err()
    );
    drop(client);
    assert_eq!(
        guard_query_count(
            &admin,
            "SELECT report_count FROM trace_skills_catalog_suggestions
              WHERE repository = 'pgtest-a/skills'"
        )
        .await,
        2
    );

    // The crawler: first-seen kept, history moves with the hash.
    let store = SkillsCatalogStore::new(admin.trace_pool_for_test());
    store.check_access().await.unwrap();
    assert!(
        store
            .pending_suggestions(1000)
            .await
            .unwrap()
            .contains(&"pgtest-a/skills".to_owned())
    );
    let repository = "pgtest-seed/repo";
    let first_crawl_at = Utc::now() - Duration::hours(2);
    let fetcher = FixtureFetcher::default().with(
        repository,
        FixtureRepository::new(HEAD_1)
            .file("skills/a/SKILL.md", skill("a", "one"))
            .file("skills/b/SKILL.md", skill("b", "one")),
    );
    let crawl = crawl_repository(&fetcher, &seed_target(repository), first_crawl_at).await;
    assert!(crawl.complete);
    store.record_crawl(&crawl, first_crawl_at).await.unwrap();

    let second_crawl_at = Utc::now() - Duration::hours(1);
    let fetcher = FixtureFetcher::default().with(
        repository,
        FixtureRepository::new(HEAD_2).file("skills/a/SKILL.md", skill("a", "two")),
    );
    let crawl = crawl_repository(&fetcher, &seed_target(repository), second_crawl_at).await;
    store.record_crawl(&crawl, second_crawl_at).await.unwrap();

    let (repositories, skills) = store.load_for_export().await.unwrap();
    let a = skills
        .iter()
        .find(|s| s.skill_id == "pgseed:skills/a/SKILL.md")
        .unwrap();
    assert_eq!(a.revision, HEAD_2);
    assert_eq!(a.previous_sha256.len(), 1);
    assert_ne!(a.previous_sha256[0], a.sha256);
    assert!((a.first_seen_at - first_crawl_at).num_seconds().abs() <= 1);
    assert!((a.last_changed_at - second_crawl_at).num_seconds().abs() <= 1);
    let b = skills
        .iter()
        .find(|s| s.skill_id == "pgseed:skills/b/SKILL.md")
        .unwrap();
    assert!(
        !b.present,
        "a complete crawl that no longer sees b marks it absent"
    );

    // An incomplete crawl removes nothing.
    let mut failing = FixtureFetcher::default().with(
        repository,
        FixtureRepository::new(HEAD_2).file("skills/a/SKILL.md", skill("a", "two")),
    );
    failing.failing_raw = vec!["skills/a/SKILL.md".into()];
    let crawl = crawl_repository(&failing, &seed_target(repository), Utc::now()).await;
    assert!(!crawl.complete);
    store.record_crawl(&crawl, Utc::now()).await.unwrap();
    let (_, skills) = store.load_for_export().await.unwrap();
    assert!(
        skills
            .iter()
            .find(|s| s.skill_id == "pgseed:skills/a/SKILL.md")
            .unwrap()
            .present
    );

    // A suggested repository's crawl settles its pending suggestions.
    let suggested = CrawlTarget {
        repository: "pgtest-a/skills".into(),
        kind: SourceKind::Suggested,
        seed: None,
        roots: Vec::new(),
    };
    let gone = FixtureFetcher::default();
    let crawl = crawl_repository(&gone, &suggested, Utc::now()).await;
    store.record_crawl(&crawl, Utc::now()).await.unwrap();
    assert_eq!(
        guard_query_count(
            &admin,
            "SELECT count(*) FROM trace_skills_catalog_suggestions
              WHERE repository = 'pgtest-a/skills' AND status = 'refused'"
        )
        .await,
        1
    );

    // The export reads what the store wrote.
    let (document, _, _) = build_document(
        &repositories
            .into_iter()
            .filter(|r| r.repository.starts_with("pgtest-"))
            .collect::<Vec<_>>(),
        &store
            .load_for_export()
            .await
            .unwrap()
            .1
            .into_iter()
            .filter(|s| s.repository.starts_with("pgtest-"))
            .collect::<Vec<_>>(),
        &Suppressions::default(),
        Environment::Staging,
        Utc::now(),
        7,
    )
    .unwrap();
    assert_eq!(
        document
            .skills
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["pgseed:skills/a/SKILL.md"]
    );
    assert_eq!(
        document.publishers[0].skills_root.as_deref(),
        Some("skills")
    );

    // The pending cap: at 10,000 pending rows a new suggestion is refused,
    // while an existing one is still counted.
    guard_execute(
        &admin,
        "INSERT INTO trace_skills_catalog_suggestions (repository, path, revision)
         SELECT 'pgtest-cap/r' || n, 'SKILL.md', '1111111111111111111111111111111111111111'
           FROM generate_series(1, 10000) AS n
         ON CONFLICT DO NOTHING",
    )
    .await;
    assert_eq!(
        runtime
            .record_skills_catalog_suggestion(&suggestion("pgtest-new/one", "SKILL.md"))
            .await
            .unwrap(),
        SkillsCatalogSuggestionOutcome::Full
    );
    assert_eq!(
        runtime
            .record_skills_catalog_suggestion(&suggestion("pgtest-cap/r1", "SKILL.md"))
            .await
            .unwrap(),
        SkillsCatalogSuggestionOutcome::Counted
    );
    guard_execute(
        &admin,
        "DELETE FROM trace_skills_catalog_suggestions WHERE repository LIKE 'pgtest-%';
         DELETE FROM trace_skills_catalog_repositories WHERE repository LIKE 'pgtest-%';",
    )
    .await;
}
