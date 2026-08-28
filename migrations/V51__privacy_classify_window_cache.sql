-- Memoized prose-PII classifications, keyed by window content hash.
--
-- Agent traces re-read the same files and echo the same tool output across
-- events, so the same window is sent to the classifier over and over.
-- Measured across 40 real contributor sessions (the machine that produced
-- most of the pilot's traces), chunked through a mirror of the production
-- chunker: 42,441 windows, 18,039 distinct -- 57.5% of classify round trips
-- re-send text already sent, at 2.35:1 duplication.
--
-- The in-process cache (PR #477) captures repeats within one driver tick.
-- This table carries them ACROSS ticks, which matters because a single tick
-- has been observed running for nine hours on a large envelope; an
-- in-memory-only cache dies before most of its value is realised.
--
-- WHAT IS STORED, AND WHAT IS NOT
--
-- `window_hash` is SHA-256 of the window text. `spans` is the classifier's
-- output for that window: character offsets and category labels, e.g.
-- [{"category":"private_email","start":12,"end":29,"score":0.99}].
--
-- **No trace text is stored, redacted or otherwise.** A cache keyed on the
-- original and storing the SCRUBBED text would be a content store; storing
-- only offsets means an attacker with this table learns the shape of a
-- classification but cannot reconstruct what was classified.
--
-- ACCEPTED TRADE-OFF: this table is a membership oracle. Anyone with read
-- access can hash a candidate string and learn whether that exact text was
-- classified for this tenant. Tenant scoping (below) bounds who shares the
-- oracle; it does not remove it. This was accepted deliberately rather than
-- overlooked -- an HMAC with a server-side secret in place of the plain hash
-- would close it at the cost of one more secret to manage, and is a
-- drop-in change if that trade is ever revisited.
--
-- TENANT SCOPED, deliberately. Of 5,075 repeated window hashes measured,
-- only 509 recurred across sessions -- and those sessions shared a tenant.
-- So scoping costs essentially nothing measurable while keeping the forced
-- RLS convention every other Trace Commons table follows. A cross-tenant
-- tier would need a promotion rule (an entry becomes global only once k
-- distinct tenants have independently classified identical text, so a hit
-- reveals "this is common" rather than "someone had this") and is
-- deliberately not built here.
--
-- INVALIDATION is by `filter_version`, part of the primary key. A model or
-- pipeline change writes under a new version and never reads stale spans.
-- Entries are therefore append-only per version; retention is handled by the
-- sweep below rather than by mutation.
CREATE TABLE IF NOT EXISTS trace_privacy_classify_cache (
    tenant_id       TEXT        NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    filter_version  TEXT        NOT NULL,
    window_hash     BYTEA       NOT NULL,
    spans           JSONB       NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, filter_version, window_hash)
);

-- The lookup is the hot path: one point read per window, potentially
-- thousands per envelope. The primary key IS the lookup index; no secondary
-- index is added, because nothing else queries this table on the read path.

-- Retention sweep support: age out fingerprints that are no longer earning
-- their keep, so the table does not accumulate content hashes indefinitely.
CREATE INDEX IF NOT EXISTS idx_trace_privacy_classify_cache_last_used
    ON trace_privacy_classify_cache (tenant_id, last_used_at);

ALTER TABLE trace_privacy_classify_cache ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_privacy_classify_cache FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_privacy_classify_cache;
CREATE POLICY trace_corpus_tenant_isolation ON trace_privacy_classify_cache
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
