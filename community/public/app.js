const config = window.TRACE_COMMONS_COMMUNITY_CONFIG || {};
const state = {
  snapshot: null,
  experience: null,
  source: "loading",
  error: null,
};

const routes = new Set([
  "/",
  "/leaderboard",
  "/install",
  "/analytics",
  "/brief",
  "/profile",
  "/about/privacy",
  "/about/data-policy",
]);

document.addEventListener("DOMContentLoaded", async () => {
  bindNavigation();
  await loadData();
  renderRoute(location.pathname);
});

window.addEventListener("popstate", () => renderRoute(location.pathname));

function bindNavigation() {
  document.addEventListener("click", (event) => {
    const link = event.target.closest("a[data-route]");
    if (!link) return;
    const url = new URL(link.href);
    if (url.origin !== location.origin) return;
    event.preventDefault();
    history.pushState({}, "", url.pathname);
    renderRoute(url.pathname);
  });
}

async function loadData() {
  await Promise.all([loadSnapshot(), loadExperience()]);
}

async function loadSnapshot() {
  const apiBase = apiOrigin();
  try {
    const response = await fetch(`${apiBase}/v1/community/leaderboard`, {
      headers: { accept: "application/json" },
    });
    if (!response.ok) throw new Error(`live snapshot ${response.status}`);
    state.snapshot = normalizeSnapshot(await response.json());
    state.source = "live";
    state.error = null;
  } catch (error) {
    try {
      const response = await fetch(config.fallbackSnapshot || "/snapshot.json", {
        headers: { accept: "application/json" },
      });
      if (!response.ok) throw new Error(`fallback snapshot ${response.status}`);
      state.snapshot = normalizeSnapshot(await response.json());
      state.source = "fallback";
      state.error = error;
    } catch (fallbackError) {
      state.snapshot = null;
      state.source = "error";
      state.error = fallbackError;
    }
  }
  updateSourceChip();
}

async function loadExperience() {
  try {
    const response = await fetch(config.experienceFeed || "/experience.json", {
      headers: { accept: "application/json" },
    });
    if (!response.ok) throw new Error(`experience ${response.status}`);
    state.experience = normalizeExperience(await response.json());
  } catch {
    state.experience = normalizeExperience({});
  }
}

function apiOrigin() {
  const params = new URLSearchParams(location.search);
  const configured = params.get("api") || config.apiBase || "https://ingest.tracecommons.ai";
  return configured.replace(/\/$/, "");
}

function updateSourceChip() {
  const chip = document.getElementById("source-chip");
  if (!chip) return;
  chip.dataset.source = state.source;
  chip.textContent =
    state.source === "live"
      ? "Live snapshot"
      : state.source === "fallback"
        ? "Preview snapshot"
        : state.source === "error"
          ? "Snapshot unavailable"
          : "Loading";
}

function renderRoute(pathname) {
  const app = document.getElementById("app");
  if (!app) return;
  const normalizedPath = normalizePathname(pathname);
  setActiveNav(normalizedPath);
  if (normalizedPath === "/install") {
    app.innerHTML = renderInstall();
    app.focus();
    return;
  }
  if (!state.snapshot) {
    app.innerHTML = renderSnapshotError();
    return;
  }
  if (normalizedPath.startsWith("/contributors/")) {
    const handle = decodeURIComponent(normalizedPath.replace("/contributors/", ""));
    app.innerHTML = renderContributor(handle);
    app.focus();
    return;
  }
  const path = routes.has(normalizedPath) ? normalizedPath : "/";
  if (path === "/analytics") {
    app.innerHTML = renderAnalytics();
  } else if (path === "/brief") {
    app.innerHTML = renderBrief();
  } else if (path === "/profile") {
    app.innerHTML = renderProfileManager();
    bindProfileForm();
  } else if (path.startsWith("/about/")) {
    app.innerHTML = renderAbout(path);
  } else {
    app.innerHTML = renderDashboard();
  }
  app.focus();
}

function normalizePathname(pathname) {
  if (!pathname || pathname === "/") return "/";
  return pathname.replace(/\/+$/, "");
}

function setActiveNav(pathname) {
  document.querySelectorAll(".nav a").forEach((link) => {
    const href = link.getAttribute("href");
    const active =
      href === pathname ||
      (href === "/leaderboard" && pathname === "/") ||
      (href !== "/" && pathname.startsWith(href));
    if (active) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  });
}

function normalizeSnapshot(snapshot) {
  const contributors = snapshot.contributors || {};
  const leaderboard = Array.isArray(snapshot.leaderboard) ? snapshot.leaderboard : [];
  return {
    ...snapshot,
    leaderboard,
    contributors,
    analytics: snapshot.analytics || {},
  };
}

function normalizeExperience(experience) {
  return {
    published_at: experience.published_at || null,
    current_prompt: experience.current_prompt || {},
    milestones: Array.isArray(experience.milestones) ? experience.milestones : [],
    weekly_rhythm: Array.isArray(experience.weekly_rhythm) ? experience.weekly_rhythm : [],
    cohort_notes: Array.isArray(experience.cohort_notes) ? experience.cohort_notes : [],
  };
}

function renderDashboard() {
  const snapshot = state.snapshot;
  const analytics = snapshot.analytics;
  const totals = summarize(snapshot);
  return `
    <section class="hero-band">
      <div>
        <p class="eyebrow">Pilot leaderboard</p>
        <h1>Pseudonymous credit for useful agent traces.</h1>
        <p class="lede">Accepted, locally scrubbed traces become visible here only after contributors opt in to a public handle. The board is ranked by rolling novelty credit.</p>
        <div class="meta-row">
          ${pill(`Window ${escapeHtml(snapshot.window || "7d")}`)}
          ${pill(`Metric ${metricLabel(snapshot.metric)}`)}
          ${pill(`Min cell ${formatInteger(snapshot.min_cell_count || 0)}`)}
        </div>
      </div>
      <div class="panel">
        <h2>Current snapshot</h2>
        <p class="lede">${formatDateTime(snapshot.computed_at)}</p>
        <div class="profile-stat-grid">
          ${statBlock("Snapshot id", shortId(snapshot.snapshot_id))}
          ${statBlock("Public handles", formatInteger(totals.contributors))}
        </div>
      </div>
    </section>

    <section class="kpi-grid" aria-label="Pilot metrics">
      ${kpi("Accepted traces", formatInteger(analytics.total_accepted), `${formatInteger(analytics.total_submissions)} submitted`)}
      ${kpi("Acceptance", formatPercent(analytics.accept_rate), `${formatInteger(analytics.total_rejected)} rejected`)}
      ${kpi("7d credit", formatNumber(totals.credit), "novelty weighted")}
      ${kpi("Top streak", topHandle(snapshot), "current handle")}
    </section>

    ${renderBriefSummary()}

    ${renderLeaderboardPanel(snapshot)}

    <section class="section-band">
      <div class="section-head">
        <div>
          <p class="eyebrow">Pilot loop</p>
          <h2>Participant rhythm</h2>
        </div>
      </div>
      <ol class="flow-list">
        <li><strong>Onboard</strong><span>Register a local device key through the invite code flow.</span></li>
        <li><strong>Submit</strong><span>Flush locally scrubbed Ironclaw traces into the pilot corpus.</span></li>
        <li><strong>Opt in</strong><span>Attach a self-declared handle to the pseudonymous contributor id.</span></li>
        <li><strong>Compare</strong><span>Watch rolling credit, accepted counts, and aggregate corpus movement.</span></li>
      </ol>
    </section>
  `;
}

function renderBriefSummary() {
  const prompt = state.experience.current_prompt || {};
  return `
    <section class="brief-strip">
      <div>
        <p class="eyebrow">${escapeHtml(prompt.window || "Pilot brief")}</p>
        <h2>${escapeHtml(prompt.title || "Trace one useful agent workflow")}</h2>
        <p class="lede">${escapeHtml(prompt.summary || "Submit a metadata-only Ironclaw trace, opt into a pseudonymous handle, and watch the next snapshot move.")}</p>
      </div>
      <div class="action-row">
        <a class="button" href="${escapeHtml(localHref(prompt.cta_href || "/profile"))}" data-route>${escapeHtml(prompt.cta_label || "Profile")}</a>
        <a class="button secondary" href="/brief" data-route>Brief</a>
      </div>
    </section>
  `;
}

function renderLeaderboardPanel(snapshot) {
  const max = Math.max(...snapshot.leaderboard.map((row) => Number(row.score) || 0), 1);
  const rows = snapshot.leaderboard
    .map((entry) => {
      const scoreValue = ((Number(entry.score) || 0) / max) * 100;
      const handle = encodeURIComponent(entry.display_handle);
      return `
        <tr>
          <td class="rank-cell">#${formatInteger(entry.rank)}</td>
          <td><a class="handle-link" href="/contributors/${handle}" data-route>${escapeHtml(entry.display_handle)}</a></td>
          <td>${formatNumber(entry.score)}</td>
          <td>${formatInteger(entry.accepted_count)}</td>
          <td>
            <meter class="meter" min="0" max="100" value="${boundedPercent(entry.accept_rate)}" aria-label="${formatPercent(entry.accept_rate)} acceptance"></meter>
          </td>
          <td>
            <meter class="meter meter-blue" min="0" max="100" value="${boundedNumber(scoreValue)}" aria-label="${formatNumber(entry.score)} score"></meter>
          </td>
        </tr>
      `;
    })
    .join("");
  return `
    <section class="panel leaderboard-panel">
      <div class="leaderboard-head">
        <div>
          <p class="eyebrow">Leaderboard</p>
          <h2>Rolling ${escapeHtml(state.snapshot.window || "7d")}</h2>
        </div>
        <a class="button secondary" href="/analytics" data-route>Analytics</a>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Rank</th>
              <th>Handle</th>
              <th>Credit</th>
              <th>Accepted</th>
              <th>Acceptance</th>
              <th>Momentum</th>
            </tr>
          </thead>
          <tbody>${rows || `<tr><td colspan="6" class="empty">No public handles in the current snapshot.</td></tr>`}</tbody>
        </table>
      </div>
    </section>
  `;
}

function renderAnalytics() {
  const analytics = state.snapshot.analytics;
  return `
    <section class="section-band">
      <div>
        <p class="eyebrow">Corpus analytics</p>
        <h1>Aggregate pilot health without trace disclosure.</h1>
        <p class="lede">These aggregates are produced by the server snapshot path. The public site renders counts already cleared by the configured release guards.</p>
      </div>
    </section>
    <section class="kpi-grid">
      ${kpi("Submissions", formatInteger(analytics.total_submissions), "window total")}
      ${kpi("Accepted", formatInteger(analytics.total_accepted), "cleared by gate")}
      ${kpi("Rejected", formatInteger(analytics.total_rejected), "held out by gate")}
      ${kpi("Accept rate", formatPercent(analytics.accept_rate), "window ratio")}
    </section>
    <section class="chart-grid">
      <div class="panel">
        <div class="section-head">
          <div>
            <p class="eyebrow">Novelty</p>
            <h2>Score distribution</h2>
          </div>
          ${pill("bucketed")}
        </div>
        ${renderHistogram(analytics.novelty_histogram || [])}
      </div>
      <div class="panel">
        <p class="eyebrow">Gate outcomes</p>
        <h2>Decision mix</h2>
        ${renderOutcomes(analytics.gate_outcomes || {})}
      </div>
    </section>
  `;
}

function renderBrief() {
  const experience = state.experience;
  const prompt = experience.current_prompt || {};
  return `
    <section class="section-band">
      <div>
        <p class="eyebrow">${escapeHtml(prompt.window || "Pilot brief")}</p>
        <h1>${escapeHtml(prompt.title || "Trace one useful agent workflow")}</h1>
        <p class="lede">${escapeHtml(prompt.summary || "Submit a metadata-only Ironclaw trace, opt into a pseudonymous handle, and watch the next snapshot move.")}</p>
        <div class="meta-row">
          ${pill(`Brief ${formatDate(experience.published_at)}`)}
          ${pill(`Snapshot ${formatDateTime(state.snapshot.computed_at)}`)}
        </div>
      </div>
    </section>

    <section class="brief-grid">
      <div class="panel prompt-panel">
        <p class="eyebrow">Current prompt</p>
        <h2>${escapeHtml(prompt.target || "Keep the first submission metadata-only so it can auto-accept.")}</h2>
        <a class="button" href="${escapeHtml(localHref(prompt.cta_href || "/profile"))}" data-route>${escapeHtml(prompt.cta_label || "Set public handle")}</a>
      </div>
      <div class="panel notes-panel">
        <p class="eyebrow">Cohort notes</p>
        ${renderNotes(experience.cohort_notes)}
      </div>
    </section>

    <section class="panel">
      <div class="section-head">
        <div>
          <p class="eyebrow">Cohort milestones</p>
          <h2>Progress from the current snapshot</h2>
        </div>
        <a class="button secondary" href="/analytics" data-route>Analytics</a>
      </div>
      ${renderMilestones(experience.milestones)}
    </section>

    <section class="section-band">
      <div class="section-head">
        <div>
          <p class="eyebrow">Weekly rhythm</p>
          <h2>Operator cadence</h2>
        </div>
      </div>
      ${renderRhythm(experience.weekly_rhythm)}
    </section>
  `;
}

function renderNotes(notes) {
  const rows = notes.length
    ? notes
    : ["The public board is opt-in identity only. Trace submission still works without publishing a handle."];
  return `<ul class="note-list">${rows.map((note) => `<li>${escapeHtml(note)}</li>`).join("")}</ul>`;
}

function renderMilestones(milestones) {
  const rows = milestones.length
    ? milestones
    : [
        {
          label: "First public cohort",
          metric: "public_handles",
          target: 5,
          unit: "handles",
          description: "Enough opted-in handles for the board to feel inhabited.",
        },
      ];
  return `
    <div class="milestone-grid">
      ${rows
        .map((milestone, index) => {
          const value = metricValue(milestone.metric);
          const target = Number(milestone.target) || 1;
          const progress = milestone.metric === "accept_rate" ? (value / target) * 100 : (value / target) * 100;
          return `
            <div class="milestone">
              <div class="milestone-head">
                <strong>${escapeHtml(milestone.label || `Milestone ${index + 1}`)}</strong>
                <span>${formatMilestoneValue(value, milestone.metric)} / ${formatMilestoneValue(target, milestone.metric)}</span>
              </div>
              <meter class="meter ${meterClass(index)}" min="0" max="100" value="${boundedNumber(progress)}" aria-label="${escapeHtml(milestone.label || "milestone")} progress"></meter>
              <p>${escapeHtml(milestone.description || milestone.unit || "")}</p>
            </div>
          `;
        })
        .join("")}
    </div>
  `;
}

function renderRhythm(rhythm) {
  const rows = rhythm.length ? rhythm : [];
  return `
    <div class="rhythm-grid">
      ${rows
        .map(
          (item) => `
            <div class="rhythm-item">
              <span>${escapeHtml(item.label || "")}</span>
              <strong>${escapeHtml(item.title || "")}</strong>
              <p>${escapeHtml(item.description || "")}</p>
            </div>
          `,
        )
        .join("")}
    </div>
  `;
}

function renderContributor(handle) {
  const profile = state.snapshot.contributors[handle];
  if (!profile) {
    return `
      <section class="section-band">
        <p class="eyebrow">Contributor</p>
        <h1>Handle not in the current snapshot.</h1>
        <p class="lede">Withdrawn profiles and contributors below the release threshold are not published.</p>
        <a class="button" href="/leaderboard" data-route>Leaderboard</a>
      </section>
    `;
  }
  return `
    <section class="profile-band">
      <div>
        <p class="eyebrow">Contributor</p>
        <h1>${escapeHtml(profile.display_handle)}</h1>
        <p class="lede">${escapeHtml(profile.bio || "No public bio.")}</p>
        <div class="meta-row">
          ${pill(`Public since ${formatDate(profile.public_since)}`)}
          ${pill(`Window ${escapeHtml(state.snapshot.window || "7d")}`)}
        </div>
      </div>
      <div class="profile-preview">
        <h2>Credit profile</h2>
        <div class="profile-stat-grid">
          ${statBlock("7d credit", formatNumber(profile.rolling_7d_credit))}
          ${statBlock("7d accepted", formatInteger(profile.rolling_7d_accepted))}
          ${statBlock("Total credit", formatNumber(profile.total_credit))}
          ${statBlock("Total accepted", formatInteger(profile.total_accepted))}
        </div>
      </div>
    </section>
    ${renderLeaderboardPanel(state.snapshot)}
  `;
}

function renderProfileManager() {
  const savedToken = sessionStorage.getItem("tracecommons_profile_token") || "";
  return `
    <section class="profile-band">
      <div>
        <p class="eyebrow">Profile</p>
        <h1>Manage your public pilot handle.</h1>
        <p class="lede">Profile changes require a short-lived upload claim carrying the public_attribution consent scope. The token is kept in this browser tab only.</p>
        <div class="notice">Use a pseudonymous handle. Do not use a legal name, email address, Slack handle, account id, or anything else that defeats the public privacy boundary.</div>
      </div>
      <form class="profile-form" id="profile-form">
        <label>
          Public attribution token
          <textarea name="token" spellcheck="false" autocomplete="off">${escapeHtml(savedToken)}</textarea>
        </label>
        <label>
          Display handle
          <input name="display_handle" maxlength="32" autocomplete="nickname" required>
        </label>
        <label>
          Bio
          <textarea name="bio" maxlength="280"></textarea>
        </label>
        <div class="action-row">
          <button type="submit">Save profile</button>
          <button type="button" class="danger" id="withdraw-profile">Withdraw</button>
        </div>
        <div class="result" id="profile-result" aria-live="polite"></div>
      </form>
    </section>
  `;
}

function bindProfileForm() {
  const form = document.getElementById("profile-form");
  const withdraw = document.getElementById("withdraw-profile");
  if (!form || !withdraw) return;
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const data = new FormData(form);
    const token = String(data.get("token") || "").trim();
    sessionStorage.setItem("tracecommons_profile_token", token);
    await profileRequest("PUT", token, {
      display_handle: String(data.get("display_handle") || "").trim(),
      bio: String(data.get("bio") || "").trim() || null,
    });
  });
  withdraw.addEventListener("click", async () => {
    const token = String(new FormData(form).get("token") || "").trim();
    sessionStorage.setItem("tracecommons_profile_token", token);
    await profileRequest("DELETE", token);
  });
}

async function profileRequest(method, token, body) {
  const result = document.getElementById("profile-result");
  if (!result) return;
  result.textContent = "Submitting";
  try {
    const response = await fetch(`${apiOrigin()}/v1/community/profile`, {
      method,
      headers: {
        accept: "application/json",
        authorization: `Bearer ${token}`,
        ...(body ? { "content-type": "application/json" } : {}),
      },
      body: body ? JSON.stringify(body) : undefined,
    });
    const payload = response.status === 204 ? null : await response.json().catch(() => null);
    if (!response.ok) {
      throw new Error((payload && payload.error) || `profile ${response.status}`);
    }
    result.textContent =
      method === "DELETE"
        ? "Profile withdrawn. The public page disappears after the next snapshot."
        : `Saved ${payload.display_handle}.`;
  } catch (error) {
    result.textContent = error.message || "Profile request failed.";
  }
}

function renderAbout(path) {
  const dataPolicy = path.endsWith("/data-policy");
  return `
    <section class="section-band">
      <p class="eyebrow">${dataPolicy ? "Data policy" : "Privacy"}</p>
      <h1>${dataPolicy ? "Public numbers come from guarded snapshots." : "Public identity is a separate opt-in."}</h1>
      <p class="lede">${dataPolicy ? "The server computes leaderboard rows from accepted submissions and profile opt-ins, then publishes only snapshot contents cleared by release gates." : "Trace submission and public display are separate choices. Accepted traces can earn credit privately even when the contributor never publishes a handle."}</p>
    </section>
    <section class="about-grid">
      <div class="panel"><h2>No raw traces</h2><p class="lede">The community site renders aggregate counts, credit totals, and handles. It does not render message text, tool payloads, or per-trace details.</p></div>
      <div class="panel"><h2>Pseudonymous handles</h2><p class="lede">A handle is self-declared public material. The server keeps the underlying contributor principal as the pseudonymous join key.</p></div>
      <div class="panel"><h2>Withdrawal</h2><p class="lede">Withdrawing stamps the profile as no longer public. The next snapshot removes the profile and contributor page from the site.</p></div>
    </section>
  `;
}

function renderInstall() {
  return `
    <section class="section-band">
      <p class="eyebrow">Install</p>
      <h1>Run the contributor on macOS, Windows, or Linux.</h1>
      <p class="lede">The desktop app and the command-line contributor are separate downloads on every platform. Nothing leaves your machine until you approve an upload.</p>
    </section>

    <section class="install-grid">
      <div class="panel">
        <h2>Windows</h2>
        <p class="lede">The app is Authenticode-signed through Azure Trusted Signing and RFC3161 timestamped.</p>
        <p class="install-label">Desktop app, keeps itself up to date</p>
        <pre class="install-code"><code>https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller</code></pre>
        <p class="install-note">Open that address in Windows. It installs the signed MSIX and Windows updates it from the same address afterwards.</p>
        <p class="install-label">Desktop app, portable</p>
        <p class="install-note">Unzip <code>trace-commons-app-windows-x86_64-*.zip</code> from the release and run <code>TraceCommons.exe</code>. It is self-contained, so there is no runtime to install first.</p>
        <p class="install-label">Command-line contributor</p>
        <pre class="install-code"><code>irm https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/install.ps1 -OutFile install.ps1
.\\install.ps1</code></pre>
        <p class="install-note">Downloading it first so you can read it is the documented form. Only an x64 build is published; Windows on Arm runs it under emulation.</p>
      </div>

      <div class="panel">
        <h2>macOS</h2>
        <p class="lede">Signed with Iqlusion Inc's Developer ID and notarized by Apple. The app is a universal build for Apple silicon and Intel, and needs macOS Sonoma or newer.</p>
        <p class="install-label">Desktop app</p>
        <pre class="install-code"><code>brew tap TraceCommons/tap
brew install --cask trace-commons</code></pre>
        <p class="install-label">Command-line contributor</p>
        <pre class="install-code"><code>brew install TraceCommons/tap/trace-commons-contributor</code></pre>
        <p class="install-note">Without Homebrew, download the DMG or the CLI zip from the release and use the shell installer below.</p>
      </div>

      <div class="panel">
        <h2>Linux</h2>
        <p class="lede">Distributed as a GPG-signed flatpak.</p>
        <p class="install-label">Desktop app</p>
        <pre class="install-code"><code>flatpak install --from https://storage.googleapis.com/tracecommons-flatpak/ai.tracecommons.Contributor.flatpakref</code></pre>
        <p class="install-label">Command-line contributor</p>
        <pre class="install-code"><code>curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/install.sh -o install.sh
sh install.sh</code></pre>
      </div>
    </section>

    <section class="section-band">
      <p class="eyebrow">New in 0.4.9</p>
      <h1>Finishing setup now finishes it.</h1>
      <p class="lede">On macOS, the last screen of setup had a button that recorded you were done and then left you sitting on that screen, as though it had not been pressed. It now takes you into the app. Windows and Linux never had this.</p>
    </section>

    <section class="about-grid">
      <div class="panel"><h2>Only the last screen</h2><p class="lede">The rest of setup worked. Connecting, choosing what you share, and picking projects all took effect as you made them. It was the single button at the end that failed to move you on.</p></div>
      <div class="panel"><h2>Nothing was lost</h2><p class="lede">If this happened to you, you were already set up -- your choices had been recorded. Quitting and reopening the app was enough to get past it, and after this update the button works the first time.</p></div>
      <div class="panel"><h2>Paths are never shown</h2><p class="lede">Projects are identified by opaque IDs and daemon-supplied labels, so a project path is never rendered anywhere in the app.</p></div>
      <div class="panel"><h2>The tray cannot send</h2><p class="lede">Waiting and armed projects, weekly totals, and quick access to Review, pause, Settings, and Quit. Tray rows are read-only and cannot approve or submit a trace.</p></div>
    </section>

    <section class="section-band">
      <p class="eyebrow">Uninstall</p>
      <h1>Leaving takes two steps, and neither is hidden.</h1>
      <p class="lede">Local state comes off first, then the program. Start everywhere with <code>trace-commons-contributor logout</code>: it stops a running daemon, then deletes the device key, config, receipts, queue, and history. Uninstalling is not withdrawal — traces you already submitted stay on the server until you withdraw them, and withdrawal needs your account session, so do it before you log out.</p>
    </section>

    <section class="install-grid">
      <div class="panel">
        <h2>Windows</h2>
        <p class="install-label">Desktop app</p>
        <pre class="install-code"><code>Get-AppxPackage Iqlusion.TraceCommons | Remove-AppxPackage</code></pre>
        <p class="install-note">That also ends the .appinstaller update subscription and the startup task. The portable build is just the unzipped folder: delete it, and if you turned on run-at-login, remove the <code>Trace Commons</code> value under <code>HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run</code>.</p>
        <p class="install-label">Command-line contributor</p>
        <pre class="install-code"><code>Remove-Item -Recurse "$env:LOCALAPPDATA\\Programs\\TraceCommons"</code></pre>
        <p class="install-note">install.ps1 appended that folder to your user PATH, so take it back out of Path under User environment variables. State lives in <code>%LOCALAPPDATA%\\trace-commons</code>, shared by the CLI and the app.</p>
      </div>

      <div class="panel">
        <h2>macOS</h2>
        <p class="install-label">Desktop app</p>
        <pre class="install-code"><code>brew uninstall --cask trace-commons</code></pre>
        <p class="install-note">Installed from the DMG instead, quit it and move <code>TraceCommons.app</code> to the Trash. Turn off run-at-login first, or clear the leftover row in System Settings, General, Login Items.</p>
        <p class="install-label">Command-line contributor</p>
        <pre class="install-code"><code>brew uninstall trace-commons-contributor</code></pre>
        <p class="install-note">From the shell installer instead, delete <code>~/.local/bin/trace-commons-contributor</code>. State lives in <code>~/Library/Application Support/trace-commons</code>.</p>
      </div>

      <div class="panel">
        <h2>Linux</h2>
        <p class="install-label">Desktop app</p>
        <pre class="install-code"><code>flatpak uninstall --delete-data ai.tracecommons.Contributor</code></pre>
        <p class="install-note">Without --delete-data the app's state stays under ~/.var/app.</p>
        <p class="install-label">Command-line contributor</p>
        <pre class="install-code"><code>systemctl --user disable --now trace-commons-contributor.service
trace-commons-contributor daemon uninstall
rm ~/.local/bin/trace-commons-contributor</code></pre>
        <p class="install-note">The first two lines matter only if you installed the background daemon. State lives in <code>~/.config/trace-commons</code>.</p>
      </div>
    </section>

    <section class="section-band">
      <p class="eyebrow">Verification</p>
      <h1>The installers refuse what they cannot verify.</h1>
      <p class="lede">Both installers check the published SHA-256, and check a signature against the platform's own roots: Authenticode naming Iqlusion Inc on Windows, a Developer ID naming our team on macOS. Neither has a force or skip-verify flag. This tool reads your coding transcripts, so an installer that can be talked out of its checks is worse than no installer. On failure you get the reason, a non-zero exit, and nothing on your PATH.</p>
    </section>
  `;
}

function renderSnapshotError() {
  return `
    <section class="section-band">
      <p class="eyebrow">Snapshot unavailable</p>
      <h1>The community snapshot could not be loaded.</h1>
      <p class="lede">${escapeHtml(state.error ? state.error.message : "Unknown error")}</p>
    </section>
  `;
}

function renderHistogram(buckets) {
  const max = Math.max(...buckets.map((bucket) => Number(bucket.count) || 0), 1);
  return `
    <div class="histogram-list" aria-label="Novelty histogram">
      ${buckets
        .map((bucket) => {
          const value = ((Number(bucket.count) || 0) / max) * 100;
          return `
            <div class="histogram-row">
              <strong>${formatBucket(bucket.bucket_micros)}</strong>
              <meter class="meter meter-blue" min="0" max="100" value="${boundedNumber(value)}" aria-label="${formatInteger(bucket.count)} traces in ${formatBucket(bucket.bucket_micros)} bucket"></meter>
              <span>${formatInteger(bucket.count)}</span>
            </div>
          `;
        })
        .join("")}
    </div>
  `;
}

function renderOutcomes(outcomes) {
  const entries = Object.entries(outcomes);
  const max = Math.max(...entries.map(([, value]) => Number(value) || 0), 1);
  return `
    <div class="outcomes">
      ${entries
        .map(([label, value], index) => {
          const width = Math.max(4, ((Number(value) || 0) / max) * 100);
          return `
            <div class="outcome-row">
              <strong>${escapeHtml(label.replaceAll("_", " "))}</strong>
              <span>${formatInteger(value)}</span>
              <meter class="meter ${meterClass(index)}" min="0" max="100" value="${boundedNumber(width)}" aria-label="${formatInteger(value)} ${escapeHtml(label.replaceAll("_", " "))} outcomes"></meter>
            </div>
          `;
        })
        .join("")}
    </div>
  `;
}

function summarize(snapshot) {
  return {
    contributors: Object.keys(snapshot.contributors || {}).length,
    credit: snapshot.leaderboard.reduce((sum, row) => sum + (Number(row.score) || 0), 0),
  };
}

function topHandle(snapshot) {
  return snapshot.leaderboard[0] ? snapshot.leaderboard[0].display_handle : "none";
}

function metricValue(metric) {
  const analytics = state.snapshot.analytics || {};
  const totals = summarize(state.snapshot);
  if (metric === "accepted_traces") return Number(analytics.total_accepted) || 0;
  if (metric === "submissions") return Number(analytics.total_submissions) || 0;
  if (metric === "accept_rate") return Number(analytics.accept_rate) || 0;
  if (metric === "public_handles") return totals.contributors;
  if (metric === "credit") return totals.credit;
  return 0;
}

function formatMilestoneValue(value, metric) {
  if (metric === "accept_rate") return formatPercent(value);
  return formatInteger(value);
}

function localHref(value) {
  const text = String(value || "/");
  return text.startsWith("/") && !text.startsWith("//") ? text : "/";
}

function kpi(label, value, sub) {
  return `<div class="kpi"><span class="label">${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong><span class="sub">${escapeHtml(sub)}</span></div>`;
}

function pill(value) {
  return `<span class="pill">${escapeHtml(value)}</span>`;
}

function statBlock(label, value) {
  return `<div class="profile-stat"><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong></div>`;
}

function boundedPercent(rate) {
  const numeric = Number(rate) || 0;
  return boundedNumber(numeric * 100);
}

function boundedNumber(value) {
  return Math.max(0, Math.min(100, Number(value) || 0));
}

function meterClass(index) {
  return ["meter-green", "meter-gold", "meter-blue", "meter-coral", "meter-violet"][index % 5];
}

function formatNumber(value) {
  return new Intl.NumberFormat("en-US", {
    maximumFractionDigits: 1,
  }).format(Number(value) || 0);
}

function formatInteger(value) {
  return new Intl.NumberFormat("en-US", {
    maximumFractionDigits: 0,
  }).format(Number(value) || 0);
}

function formatPercent(value) {
  return `${Math.round((Number(value) || 0) * 100)}%`;
}

function formatDateTime(value) {
  if (!value) return "No timestamp";
  return new Intl.DateTimeFormat("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}

function formatDate(value) {
  if (!value) return "unknown";
  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date(value));
}

function formatBucket(value) {
  const numeric = Number(value) || 0;
  return `${Math.round(numeric / 10000) / 100}`;
}

function metricLabel(metric) {
  return String(metric || "novelty_credit").replaceAll("_", " ");
}

function shortId(value) {
  const text = String(value || "");
  return text.length > 12 ? `${text.slice(0, 8)}...${text.slice(-4)}` : text || "none";
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
