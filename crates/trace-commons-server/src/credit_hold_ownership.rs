// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Resolve credit holds through the same account ownership as settlement.

use crate::db::Database;
use crate::error::DatabaseError;
use std::collections::BTreeSet;
use tokio_postgres::Transaction;

/// Expand active hold references to every credit identity owned by the held
/// account. Retain raw references so unlinked principals remain held directly.
pub async fn expand_credit_hold_ownership(
    db: &dyn Database,
    tenant: &str,
    mut refs: BTreeSet<String>,
) -> Result<BTreeSet<String>, DatabaseError> {
    if refs.is_empty() {
        return Ok(refs);
    }
    let principals: Vec<_> = refs
        .iter()
        .filter(|r| crate::db::settlement_key_account_id(r).is_none())
        .cloned()
        .collect();
    let owners = db
        .resolve_principals_to_accounts(tenant, &principals)
        .await?;
    let accounts: BTreeSet<_> = refs
        .iter()
        .filter_map(|r| crate::db::settlement_key_account_id(r))
        .chain(owners.into_values())
        .collect();
    for account in accounts {
        refs.insert(format!("account:{account}"));
        refs.extend(
            db.expand_account_principals(tenant, account)
                .await?
                .to_vec(),
        );
    }
    Ok(refs)
}

/// The pipeline's under-lock recheck uses this transaction's ownership snapshot.
pub(crate) async fn active_credit_holds_on_tx(
    tx: &Transaction<'_>,
    tenant: &str,
) -> Result<BTreeSet<String>, DatabaseError> {
    let rows = tx.query(
        "SELECT credit_account_ref FROM trace_credit_holds WHERE tenant_id=$1 AND released_at IS NULL",
        &[&tenant],
    ).await?;
    let mut refs: BTreeSet<String> = rows.into_iter().map(|r| r.get(0)).collect();
    if refs.is_empty() {
        return Ok(refs);
    }
    // Use exactly the parser used by the pre-effect check, including compact,
    // uppercase, braced and URN UUID spellings. SQL string equality disagrees.
    let accounts: Vec<_> = refs
        .iter()
        .filter_map(|r| crate::db::settlement_key_account_id(r))
        .collect();
    let principals: Vec<_> = refs
        .iter()
        .filter(|r| crate::db::settlement_key_account_id(r).is_none())
        .cloned()
        .collect();
    let rows = tx
        .query(
            "WITH held_accounts AS (
            SELECT account_id FROM trace_account_owned_principals
            WHERE tenant_id=$1 AND principal_ref=ANY($2)
            UNION SELECT unnest($3::uuid[])
        )
        SELECT 'account:' || account_id::text AS principal_ref FROM held_accounts
        UNION SELECT p.principal_ref FROM trace_account_owned_principals p
        JOIN held_accounts a ON a.account_id=p.account_id
        WHERE p.tenant_id=$1",
            &[&tenant, &principals, &accounts],
        )
        .await?;
    refs.extend(rows.into_iter().map(|r| r.get::<_, String>(0)));
    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_trust::{
        device_claim_actor, device_principal_ref, device_upload_principal_refs,
    };
    use crate::config::DatabaseConfig;
    use crate::db::postgres::PgBackend;
    use uuid::Uuid;

    #[tokio::test]
    async fn credit_holds_follow_account_and_alias_ownership_in_both_read_paths() {
        let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
            eprintln!("SKIPPED: isolated TRACE_COMMONS_PG_TEST_DATABASE_URL required");
            return;
        };
        assert!(url.contains("127.0.0.1"));
        let db = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4))
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let mut conn = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
        let tenant = format!("credit-hold-{}", Uuid::new_v4());
        let account = Uuid::new_v4();
        let device = format!(
            "sha256:{}",
            uuid::Uuid::new_v4().simple().to_string().repeat(2)
        );
        let parent = device_principal_ref(&tenant, &device);
        let actor_a = device_claim_actor(&tenant, &device, Some("subject-a"));
        let actor_b = device_claim_actor(&tenant, &device, Some("subject-b"));
        let alias_a = device_upload_principal_refs(&tenant, &device, &actor_a).unwrap()[0].clone();
        let alias_b = device_upload_principal_refs(&tenant, &device, &actor_b).unwrap()[0].clone();
        let key = format!("account:{account}");
        conn.execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1)",
            &[&tenant],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO device_keys(tenant_id,device_key_id,public_key,invite_subject_hash) VALUES($1,$2,'test',$2)", &[&tenant,&device]).await.unwrap();
        conn.execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
        conn.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)", &[&tenant,&account,&parent]).await.unwrap();
        conn.execute("INSERT INTO trace_near_account_anchors(tenant_id,account_id,anchor_hash,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,'{}',$3,$3)", &[&tenant,&account,&device]).await.unwrap();
        conn.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,account_id,principal_ref,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$4)", &[&tenant,&account,&parent,&device]).await.unwrap();
        db.bind_device_upload_principal(&tenant, &device, &actor_a)
            .await
            .unwrap();
        db.bind_device_upload_principal(&tenant, &device, &actor_b)
            .await
            .unwrap();
        assert_eq!(
            expand_credit_hold_ownership(&db, &tenant, BTreeSet::from(["unlinked".into()]))
                .await
                .unwrap(),
            BTreeSet::from(["unlinked".into()])
        );
        assert!(
            expand_credit_hold_ownership(&db, &tenant, BTreeSet::new())
                .await
                .unwrap()
                .is_empty()
        );
        let other_tenant = format!("other-{tenant}");
        conn.execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1)",
            &[&other_tenant],
        )
        .await
        .unwrap();
        assert_eq!(
            expand_credit_hold_ownership(&db, &other_tenant, BTreeSet::from([alias_a.clone()]))
                .await
                .unwrap(),
            BTreeSet::from([alias_a.clone()])
        );
        let upper = format!("account:{}", account.to_string().to_uppercase());
        let compact = format!("account:{}", account.simple());
        let braced = format!("account:{}", account.braced());
        let urn = format!("account:{}", account.urn());
        for held in [&alias_a, &key, &upper, &compact, &braced, &urn] {
            let expanded =
                expand_credit_hold_ownership(&db, &tenant, BTreeSet::from([held.clone()]))
                    .await
                    .unwrap();
            assert!(
                expanded.contains(&alias_a),
                "held account must include first upload"
            );
            assert!(
                expanded.contains(&alias_b),
                "held account must include sibling upload"
            );
            assert!(
                expanded.contains(&key),
                "held account must include settlement key"
            );
            assert!(!expanded.contains("unrelated"));
            let hold_id = Uuid::new_v4();
            conn.execute("INSERT INTO trace_credit_holds(tenant_id,hold_id,credit_account_ref,credit_account_hash,reason,reason_hash,actor_principal_ref) VALUES($1,$2,$3,'hash','suspected_abuse','hash','operator')", &[&tenant,&hold_id,held]).await.unwrap();
            let tx = conn.transaction().await.unwrap();
            tx.execute(
                "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
                &[&tenant],
            )
            .await
            .unwrap();
            let locked = active_credit_holds_on_tx(&tx, &tenant).await.unwrap();
            assert_eq!(locked, expanded);
            tx.commit().await.unwrap();
            conn.execute(
                "UPDATE trace_credit_holds SET released_at=NOW() WHERE tenant_id=$1 AND hold_id=$2",
                &[&tenant, &hold_id],
            )
            .await
            .unwrap();
        }
        let tx = conn.transaction().await.unwrap();
        assert!(
            active_credit_holds_on_tx(&tx, &tenant)
                .await
                .unwrap()
                .is_empty()
        );
        tx.commit().await.unwrap();
    }
}
