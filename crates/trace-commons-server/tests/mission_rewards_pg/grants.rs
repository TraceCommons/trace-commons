// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: pins the reward table privilege matrix against the deployed catalog.

use crate::fixture::RewardPgFixture;

// The six tables V69 creates. Every other reward table is discovered from the
// catalog below, so a later migration cannot add one that nothing checks.
const V69_REWARD_TABLES: [&str; 6] = [
    "trace_reward_awards",
    "trace_reward_decisions",
    "trace_reward_invalidations",
    "trace_reward_operators",
    "trace_reward_programs",
    "trace_reward_reservations",
];

const PRIVILEGES: [&str; 4] = ["DELETE", "INSERT", "SELECT", "UPDATE"];

// (table, privilege, table-wide, any column). The runtime login reaches every
// row through a SECURITY DEFINER function owned by the guard, so it holds
// nothing here; the guard holds exactly what those functions execute. A widened
// GRANT in any reward migration changes one of these cells.
fn expected_guard_matrix() -> Vec<(&'static str, &'static str, bool, bool)> {
    let mut expected = Vec::new();
    for table in V69_REWARD_TABLES {
        for privilege in PRIVILEGES {
            let granted = match (table, privilege) {
                (_, "SELECT") => (true, true),
                ("trace_reward_operators", "INSERT") => (false, false),
                (_, "INSERT") => (true, true),
                // Six named columns only, so the table-wide answer stays false.
                ("trace_reward_reservations", "UPDATE") => (false, true),
                (_, "UPDATE") => (false, false),
                (_, _) => (false, false),
            };
            expected.push((table, privilege, granted.0, granted.1));
        }
    }
    expected
}

async fn privilege_matrix(
    admin: &tokio_postgres::Client,
    role: &str,
    tables: &[String],
) -> Vec<(String, String, bool, bool)> {
    admin
        .query(
            "SELECT c.relname, p.priv, \
                    pg_catalog.has_table_privilege($1::name, c.oid, p.priv), \
                    CASE WHEN p.priv = 'DELETE' THEN FALSE ELSE EXISTS ( \
                        SELECT 1 FROM pg_catalog.pg_attribute a \
                         WHERE a.attrelid = c.oid AND a.attnum > 0 \
                           AND NOT a.attisdropped \
                           AND pg_catalog.has_column_privilege( \
                                   $1::name, c.oid, a.attnum, p.priv) \
                    ) END \
               FROM pg_catalog.pg_class c \
               CROSS JOIN (VALUES ('DELETE'), ('INSERT'), ('SELECT'), ('UPDATE')) \
                    AS p(priv) \
              WHERE c.relnamespace = 'public'::pg_catalog.regnamespace \
                AND c.relkind = 'r' AND c.relname = ANY($2::text[]) \
              ORDER BY c.relname, p.priv",
            &[&role, &tables],
        )
        .await
        .expect("read reward privilege matrix")
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn reward_table_privileges_are_pinned_for_the_runtime_and_guard_roles() {
    let fixture = RewardPgFixture::new().await;
    let discovered: Vec<String> = fixture
        .admin
        .query(
            "SELECT c.relname FROM pg_catalog.pg_class c \
              WHERE c.relnamespace = 'public'::pg_catalog.regnamespace \
                AND c.relkind = 'r' AND c.relname LIKE 'trace\\_reward\\_%' \
              ORDER BY c.relname",
            &[],
        )
        .await
        .expect("discover reward tables")
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect();
    for table in V69_REWARD_TABLES {
        assert!(
            discovered.iter().any(|name| name == table),
            "{table} must exist after the full migration chain"
        );
    }

    // The runtime login and the role it inherits hold nothing on any reward
    // table, including tables later migrations add.
    for role in [fixture.issuer.as_str(), "trace_reward_runtime"] {
        for (table, privilege, table_wide, any_column) in
            privilege_matrix(&fixture.admin, role, &discovered).await
        {
            assert!(
                !table_wide && !any_column,
                "{role} must hold no {privilege} on {table}: \
                 table-wide {table_wide}, column {any_column}"
            );
        }
    }

    let guard_tables: Vec<String> = V69_REWARD_TABLES.iter().map(|t| (*t).to_owned()).collect();
    let actual = privilege_matrix(&fixture.admin, "trace_reward_guard", &guard_tables).await;
    let expected: Vec<(String, String, bool, bool)> = expected_guard_matrix()
        .into_iter()
        .map(|(table, privilege, table_wide, any_column)| {
            (
                table.to_owned(),
                privilege.to_owned(),
                table_wide,
                any_column,
            )
        })
        .collect();
    assert_eq!(
        actual, expected,
        "the guard role's reward table privileges changed"
    );

    // The behavioural companion: the same denial through the wire, per table.
    let runtime = fixture.runtime().await;
    for table in V69_REWARD_TABLES {
        for statement in [
            format!("SELECT tenant_id FROM public.{table} LIMIT 1"),
            format!("INSERT INTO public.{table} SELECT * FROM public.{table} WHERE FALSE"),
            format!("UPDATE public.{table} SET tenant_id = tenant_id WHERE FALSE"),
            format!("DELETE FROM public.{table} WHERE FALSE"),
        ] {
            assert!(
                runtime.execute(statement.as_str(), &[]).await.is_err(),
                "runtime direct DML must be denied: {statement}"
            );
        }
    }
}
