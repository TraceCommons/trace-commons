// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Durable per-submission admission and conservative processing-cost reservations.
//! A cost bound is an operator-configured unit, never a fabricated USD conversion.
use crate::{db::postgres::PgBackend, error::DatabaseError};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Clone)]
pub struct AdmissionLimits {
    pub window_attempts: i64,
    pub account_cost_limit: i64,
    pub global_cost_limit: i64,
    pub processing_cost_bound: i64,
    pub lease_seconds: i64,
    pub challenge_ttl_seconds: i64,
}
impl AdmissionLimits {
    pub fn from_env() -> Result<Option<Self>, &'static str> {
        match std::env::var("TRACE_COMMONS_ADMISSION_ENABLED").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("false") | Ok("0") => return Ok(None),
            Ok("true") | Ok("1") => {}
            _ => return Err("admission_configuration_invalid"),
        }
        let integer = |key| {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or("admission_configuration_missing")
        };
        let limits = Self {
            window_attempts: integer("TRACE_COMMONS_ADMISSION_WINDOW_ATTEMPTS")?,
            account_cost_limit: integer("TRACE_COMMONS_ADMISSION_ACCOUNT_COST_LIMIT")?,
            global_cost_limit: integer("TRACE_COMMONS_ADMISSION_GLOBAL_COST_LIMIT")?,
            processing_cost_bound: integer("TRACE_COMMONS_ADMISSION_PROCESSING_COST_BOUND")?,
            lease_seconds: integer("TRACE_COMMONS_ADMISSION_LEASE_SECONDS")?,
            challenge_ttl_seconds: integer("TRACE_COMMONS_ADMISSION_CHALLENGE_TTL_SECONDS")?,
        };
        if limits.window_attempts < 0
            || limits.account_cost_limit <= 0
            || limits.global_cost_limit <= 0
            || limits.processing_cost_bound <= 0
            || !(1..=86400).contains(&limits.lease_seconds)
            || !(1..=86400).contains(&limits.challenge_ttl_seconds)
        {
            return Err("admission_configuration_invalid");
        }
        Ok(Some(limits))
    }
}

#[derive(Clone)]
pub struct AdmissionReservation {
    pub tenant_id: String,
    pub anchor_hash: String,
    pub submission_id: Uuid,
    pub body_hash: String,
    pub receipt_hash: Option<String>,
    pub challenge_hash: Option<String>,
    pub lease_id: Uuid,
    pub limits: AdmissionLimits,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionDecision {
    Reserved,
    Completed,
    Busy,
    Exhausted,
    Conflict,
    Refused,
}
fn database_refused() -> DatabaseError {
    DatabaseError::Pool("admission_database_unavailable".into())
}

/// Owns an isolated database session for one in-flight submission. Dropping the
/// guard removes its connection from the pool and closes it, including cancellation;
/// a session advisory lock can never leak into a recycled pooled connection.
pub struct AdmissionProcessingGuard(Option<deadpool_postgres::ClientWrapper>);
impl Drop for AdmissionProcessingGuard {
    fn drop(&mut self) {
        if let Some(client) = self.0.take() {
            drop(client);
        }
    }
}

impl PgBackend {
    pub(crate) async fn check_admission_runtime(&self) -> Result<bool, DatabaseError> {
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let row=client.query_one(r#"
        SELECT NOT r.rolsuper AND NOT r.rolbypassrls
          AND NOT pg_has_role(current_user,'trace_admission_guard','MEMBER')
          AND NOT COALESCE(pg_has_role(current_user,to_regrole('trace_onboarding_retention_guard'),'MEMBER'),FALSE)
          AND NOT has_table_privilege(current_user,'public.trace_admission_receipts','SELECT,INSERT,UPDATE,DELETE')
          AND NOT has_table_privilege(current_user,'public.trace_admission_global_budget','SELECT,INSERT,UPDATE,DELETE')
          AND has_function_privilege(current_user,'public.trace_reserve_admission(text,text,uuid,text,text,text,bigint,bigint,bigint,bigint,uuid,bigint)','EXECUTE')
          AND has_function_privilege(current_user,'public.trace_transition_admission(text,uuid,uuid,text)','EXECUTE')
          AND (SELECT bool_and(c.relrowsecurity AND c.relforcerowsecurity AND c.relowner<>r.oid)
            FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE n.nspname='public' AND c.relname IN ('trace_admission_challenges','trace_admission_accounts','trace_admission_submissions','trace_admission_receipts','trace_admission_global_budget'))
          AND (SELECT bool_and(has_table_privilege(current_user,t,p)) FROM
            unnest(ARRAY['public.trace_admission_challenges','public.trace_admission_accounts','public.trace_admission_submissions']) t
            CROSS JOIN unnest(ARRAY['SELECT','INSERT','UPDATE']) p)
        FROM pg_roles r WHERE r.rolname=current_user
        "#,&[]).await.map_err(|_|database_refused())?;
        Ok(row.get::<_, Option<bool>>(0).unwrap_or(false))
    }
    pub(crate) async fn completed_admission(
        &self,
        tenant: &str,
        anchor: &str,
        submission: Uuid,
        body_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let found=tx.query_opt("SELECT 1 FROM trace_admission_submissions WHERE tenant_id=$1 AND anchor_hash=$2 AND submission_id=$3 AND body_hash=$4 AND status='completed'", &[&tenant,&anchor,&submission,&body_hash]).await.map_err(|_|database_refused())?.is_some();
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(found)
    }
    pub(crate) async fn lock_admission(
        &self,
        tenant: &str,
        submission: Uuid,
    ) -> Result<Option<AdmissionProcessingGuard>, DatabaseError> {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"trace-admission-processing.v1:");
        hash.update((tenant.len() as u64).to_be_bytes());
        hash.update(tenant.as_bytes());
        hash.update(submission.as_bytes());
        let digest = hash.finalize();
        let key = i64::from_be_bytes(digest[..8].try_into().map_err(|_| database_refused())?);
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let acquired: bool = client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&key])
            .await
            .map_err(|_| database_refused())?
            .get(0);
        Ok(acquired
            .then(|| AdmissionProcessingGuard(Some(deadpool_postgres::Object::take(client)))))
    }

    pub(crate) async fn insert_admission_challenge(
        &self,
        tenant: &str,
        anchor: &str,
        challenge: &str,
        expires: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        tx.execute("INSERT INTO trace_admission_challenges(tenant_id,anchor_hash,challenge_hash,expires_at) VALUES($1,$2,$3,$4)",
            &[&tenant,&anchor,&challenge,&expires]).await.map_err(|_| database_refused())?;
        tx.commit().await.map_err(|_| database_refused())
    }
    pub(crate) async fn reserve_admission(
        &self,
        r: &AdmissionReservation,
    ) -> Result<AdmissionDecision, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &r.tenant_id).await?;
        let row = tx
            .query_one(
                "SELECT trace_reserve_admission($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
                &[
                    &r.tenant_id,
                    &r.anchor_hash,
                    &r.submission_id,
                    &r.body_hash,
                    &r.receipt_hash,
                    &r.challenge_hash,
                    &r.limits.window_attempts,
                    &r.limits.account_cost_limit,
                    &r.limits.global_cost_limit,
                    &r.limits.processing_cost_bound,
                    &r.lease_id,
                    &r.limits.lease_seconds,
                ],
            )
            .await
            .map_err(|_| database_refused())?;
        let decision = match row.get::<_, &str>(0) {
            "reserved" => AdmissionDecision::Reserved,
            "completed" => AdmissionDecision::Completed,
            "busy" => AdmissionDecision::Busy,
            "budget_exhausted" | "window_exhausted" => AdmissionDecision::Exhausted,
            "conflict" => AdmissionDecision::Conflict,
            _ => AdmissionDecision::Refused,
        };
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(decision)
    }
    pub(crate) async fn transition_admission(
        &self,
        tenant: &str,
        submission: Uuid,
        lease: Uuid,
        next: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| database_refused())?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let changed = tx
            .query_one(
                "SELECT trace_transition_admission($1,$2,$3,$4)",
                &[&tenant, &submission, &lease, &next],
            )
            .await
            .map_err(|_| database_refused())?
            .get(0);
        tx.commit().await.map_err(|_| database_refused())?;
        Ok(changed)
    }
}
