// INTEGRATION: loaded after app.js and owns reviewed /runs/:slug pages.

function updatePublicRunChip() {
  const chip = document.getElementById("source-chip");
  if (!chip) return;
  chip.dataset.source = "public-run";
  chip.textContent =
    state.publicRunStatus === "ready"
      ? "Reviewed page"
      : state.publicRunStatus === "not-found" || state.publicRunStatus === "error"
        ? "Page unavailable"
        : "Loading page";
}

function publicRunSlugFromPath(pathname) {
  const normalizedPath = normalizePathname(pathname);
  if (!normalizedPath.startsWith("/runs/")) return null;
  const encoded = normalizedPath.slice("/runs/".length);
  if (!encoded || encoded.includes("/")) return null;
  try {
    return decodeURIComponent(encoded);
  } catch {
    return encoded;
  }
}

function publicRunApiOrigin() {
  return (config.apiBase || "https://ingest.tracecommons.ai").replace(/\/$/, "");
}

async function loadPublicRun(slug) {
  const request = ++state.publicRunRequest;
  state.publicRunSlug = slug;
  state.publicRun = null;
  state.publicRunStatus = "loading";
  state.publicRunError = null;
  renderCurrentPublicRun(slug);
  if (!/^[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?$/.test(slug)) {
    state.publicRunStatus = "not-found";
    renderCurrentPublicRun(slug);
    return;
  }
  try {
    const response = await fetch(`${publicRunApiOrigin()}/v1/community/runs/${encodeURIComponent(slug)}`, {
      headers: { accept: "application/json" },
    });
    if (request !== state.publicRunRequest || state.publicRunSlug !== slug) return;
    if (response.status === 404) {
      state.publicRunStatus = "not-found";
    } else if (!response.ok) {
      throw new Error(`public workflow ${response.status}`);
    } else {
      const run = normalizePublicRun(await response.json());
      if (request !== state.publicRunRequest || state.publicRunSlug !== slug) return;
      state.publicRun = run;
      state.publicRunStatus = "ready";
    }
  } catch (error) {
    if (request !== state.publicRunRequest || state.publicRunSlug !== slug) return;
    state.publicRunStatus = "error";
    state.publicRunError = error;
  }
  renderCurrentPublicRun(slug);
}

function renderCurrentPublicRun(slug) {
  if (publicRunSlugFromPath(location.pathname) === slug) renderRoute(location.pathname);
}

function normalizePublicRun(run) {
  return {
    slug: String(run.slug || ""),
    title: String(run.title || ""),
    outcome_summary: String(run.outcome_summary || ""),
    correction_excerpt: run.correction_excerpt ? String(run.correction_excerpt) : null,
    workflow: String(run.workflow || ""),
    reuse_permission: String(run.reuse_permission || ""),
    evidence: Array.isArray(run.evidence) ? run.evidence : [],
    task_success: String(run.task_success || ""),
    contributed_version: String(run.contributed_version || ""),
    version: Number(run.version) || 1,
    published_at: run.published_at || null,
    source: run.source || null,
    source_unavailable: Boolean(run.source_unavailable),
    variations: Array.isArray(run.variations) ? run.variations : [],
  };
}

function renderPublicRunRoute(slug) {
  if (state.publicRunSlug !== slug || state.publicRunStatus === "idle" || state.publicRunStatus === "loading") {
    return `
      <section class="section-band public-run-status">
        <p class="eyebrow">Published workflow</p>
        <h1>Reading the reviewed page.</h1>
      </section>
    `;
  }
  if (state.publicRunStatus === "not-found") {
    return `
      <section class="section-band public-run-status">
        <p class="eyebrow">Published workflow</p>
        <h1>This page is unavailable.</h1>
        <p class="lede">The creator may have unpublished it, or the address may be incomplete.</p>
        <a class="button" href="/" data-route>Leaderboard</a>
      </section>
    `;
  }
  if (state.publicRunStatus === "error" || !state.publicRun) {
    return `
      <section class="section-band public-run-status">
        <p class="eyebrow">Published workflow</p>
        <h1>The page could not be read.</h1>
        <p class="lede">${escapeHtml(state.publicRunError?.message || "Retry the page.")}</p>
        <button type="button" data-retry-public-run>Retry page</button>
      </section>
    `;
  }
  return renderPublicRun(state.publicRun);
}

function renderPublicRun(run) {
  const evidence = run.evidence
    .map((item) => `<li>${escapeHtml(item.excerpt || "")}</li>`)
    .join("");
  const correction = run.correction_excerpt
    ? `<section class="public-run-section"><p class="eyebrow">Decisive correction</p><p class="public-run-prose">${escapeHtml(run.correction_excerpt)}</p></section>`
    : "";
  const source = run.source
    ? `<a href="/runs/${encodeURIComponent(run.source.slug || "")}" data-route>${escapeHtml(run.source.title || "Source workflow")}</a>`
    : run.source_unavailable
      ? "Source workflow unavailable"
      : "First published version";
  const variations = run.variations.length
    ? `<ul class="public-run-links">${run.variations
        .map(
          (variation) =>
            `<li><a href="/runs/${encodeURIComponent(variation.slug || "")}" data-route>${escapeHtml(variation.title || "Variation")}</a></li>`,
        )
        .join("")}</ul>`
    : `<p class="lede">No linked variations have been published.</p>`;
  const license = reusePermission(run.reuse_permission);
  return `
    <section class="public-run-hero">
      <div>
        <p class="eyebrow">Published workflow</p>
        <h1 class="public-run-title">${escapeHtml(run.title)}</h1>
        <p class="lede public-run-outcome">${escapeHtml(run.outcome_summary)}</p>
        <div class="meta-row">
          ${pill(`Creator report ${taskSuccessLabel(run.task_success)}`)}
          ${pill(`Contributed ${run.contributed_version}`)}
          ${pill(`Version ${formatInteger(run.version)}`)}
        </div>
      </div>
      <div class="public-run-action">
        <button type="button" data-use-workflow>Use workflow</button>
        <p class="result" data-workflow-result aria-live="polite">Copies the instructions with this source page.</p>
      </div>
    </section>

    <section class="public-run-grid">
      <div class="panel public-run-main">
        <section class="public-run-section">
          <p class="eyebrow">Reusable instructions</p>
          <p class="public-run-workflow">${escapeHtml(run.workflow)}</p>
        </section>
        ${correction}
        <section class="public-run-section">
          <p class="eyebrow">Observed evidence</p>
          <ul class="public-run-evidence">${evidence}</ul>
        </section>
      </div>
      <aside class="panel public-run-provenance">
        <p class="eyebrow">Reuse permission</p>
        <h2>${escapeHtml(license.label)}</h2>
        <p class="lede">${escapeHtml(license.explanation)}</p>
        <a href="${license.url}" rel="license noreferrer">Read permission</a>
        <div class="public-run-divider"></div>
        <p class="eyebrow">Source</p>
        <p>${source}</p>
        <p class="public-run-date">Published ${formatDate(run.published_at)}</p>
      </aside>
    </section>

    <section class="section-band">
      <div class="section-head">
        <div>
          <p class="eyebrow">Variations</p>
          <h2>Workflows built from this source</h2>
        </div>
      </div>
      ${variations}
    </section>
  `;
}

function reusePermission(value) {
  if (value === "cc0_1_0") {
    return {
      label: "CC0 1.0",
      explanation: "The creator permits reuse without attribution.",
      url: "https://creativecommons.org/publicdomain/zero/1.0/",
    };
  }
  return {
    label: "CC BY 4.0",
    explanation: "The creator permits reuse with attribution.",
    url: "https://creativecommons.org/licenses/by/4.0/",
  };
}

function taskSuccessLabel(value) {
  if (value === "success") return "completed";
  if (value === "partial") return "partly completed";
  if (value === "failure") return "did not complete";
  return "not reported";
}

async function copyPublicWorkflow(button) {
  const run = state.publicRun;
  const result = document.querySelector("[data-workflow-result]");
  if (!run || state.publicRunStatus !== "ready") return;
  const sourceUrl = `${location.origin}/runs/${encodeURIComponent(run.slug)}`;
  const text = `${run.workflow}\n\nSource: ${sourceUrl}`;
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
    } else {
      const buffer = document.createElement("textarea");
      buffer.className = "clipboard-buffer";
      buffer.value = text;
      document.body.append(buffer);
      buffer.select();
      if (!document.execCommand("copy")) throw new Error("copy unavailable");
      buffer.remove();
    }
    button.textContent = "Workflow copied";
    if (result) result.textContent = "Instructions and source copied.";
  } catch {
    if (result) result.textContent = "Copy failed. Select the reusable instructions instead.";
  }
}
