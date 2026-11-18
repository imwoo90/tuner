import { escapeHtml, renderMarkdown } from "/dashboard/assets/markdown.js";
import { t, onLangChange, getLang } from "/dashboard/assets/i18n.js";
import { createOnboardingRenderer } from "/dashboard/assets/onboarding.js";

export function createInspectorController({ apiFetch, getActiveSessionKey, onForceLogout, onViewChange }) {
  const container = document.getElementById("inspector-content");
  const tabsNav = document.getElementById("inspector-tabs");
  const inspectorEl = document.getElementById("inspector");
  const toggleBtn = document.getElementById("btn-toggle-inspector");

  let activeTab = "preview";
  let currentPreviewPath = "";
  let previewRawMode = false;
  let activeHitlToken = "";
  let lastDoneSiteKey = "";
  let renderSeq = 0;

  function showLoading(labelKey) {
    container.innerHTML = `<div class="inspector-loading" role="status" aria-live="polite">
      <span class="loading-spinner" aria-hidden="true"></span>
      <span>${escapeHtml(t(labelKey))}</span>
    </div>`;
  }

  function syncToggleState() {
    const open = window.innerWidth < 1024
      ? inspectorEl.classList.contains("open")
      : !inspectorEl.classList.contains("collapsed");
    toggleBtn?.classList.toggle("active", open);
    onViewChange?.();
  }

  function openPanel() {
    if (window.innerWidth < 1024) {
      inspectorEl.classList.add("open");
      document.getElementById("sidebar")?.classList.remove("open");
      document.getElementById("drawer-backdrop")?.classList.remove("hidden");
    } else {
      inspectorEl.classList.remove("collapsed");
    }
    syncToggleState();
  }

  function closePanel() {
    if (window.innerWidth < 1024) {
      inspectorEl.classList.remove("open");
      document.getElementById("drawer-backdrop")?.classList.add("hidden");
    } else {
      inspectorEl.classList.add("collapsed");
    }
    syncToggleState();
  }

  function togglePanel() {
    if (window.innerWidth < 1024) {
      if (inspectorEl.classList.contains("open")) closePanel();
      else {
        openPanel();
        refreshCurrentTab();
      }
    } else {
      if (inspectorEl.classList.contains("collapsed")) {
        openPanel();
        refreshCurrentTab();
      } else {
        closePanel();
      }
    }
  }

  function isCollapsed() {
    return window.innerWidth < 1024
      ? !inspectorEl.classList.contains("open")
      : inspectorEl.classList.contains("collapsed");
  }

  function setTab(tabName) {
    activeTab = tabName;
    inspectorEl.classList.toggle("wide-browser", tabName === "browser");
    tabsNav.querySelectorAll(".tab-btn").forEach((btn) => {
      btn.classList.toggle("active", btn.dataset.tab === tabName);
    });
    onViewChange?.();
    refreshCurrentTab();
  }

  function openTab(tabName) {
    openPanel();
    setTab(tabName);
  }

  function openFilePreview(filePath) {
    currentPreviewPath = filePath;
    previewRawMode = false;
    openTab("preview");
  }

  async function renderTreeTab() {
    const seq = ++renderSeq;
    showLoading("loadingTree");
    const sk = getActiveSessionKey() || "";
    const res = await apiFetch(`/dashboard/api/subagents?session_key=${encodeURIComponent(sk)}`);
    if (seq !== renderSeq || activeTab !== "tree") return;
    if (!res.ok) {
      container.innerHTML = `<div class="alert alert-danger">${escapeHtml(t("treeLoadError", { status: res.status }))}</div>`;
      return;
    }
    const data = await res.json();
    if (seq !== renderSeq || activeTab !== "tree") return;
    const wp = data.warm_pools || {};
    let html = `<div class="card">
      <div class="card-row"><strong>${escapeHtml(t("warmPoolsTitle"))}</strong><button id="btn-refresh-tree" class="btn btn-sm" type="button">${escapeHtml(t("refresh"))}</button></div>
      <div class="card-row">
        <span class="pill">agy ${Number(wp.agy || 0)}</span>
        <span class="pill">claude ${Number(wp.claude || 0)}</span>
        <span class="pill">codex ${Number(wp.codex || 0)}</span>
      </div>`;
    if (data.session_pids) {
      const sp = data.session_pids;
      html += `<details class="cot-block"><summary>${escapeHtml(t("pidDetails"))}</summary><div class="cot-body mono">agy: ${escapeHtml(sp.agy || "-")} · claude: ${escapeHtml(sp.claude || "-")} · codex: ${escapeHtml(sp.codex || "-")}</div></details>`;
    }
    html += `</div>`;

    const nodes = data.nodes || [];
    const totalSubs = nodes.reduce((acc, n) => acc + (n.subagents?.length || 0) + (n.swarm_members?.length || 0), 0);
    if (nodes.length === 0 || (nodes.length === 1 && totalSubs === 0 && !(nodes[0].tasks?.length))) {
      html += `<div class="card"><div class="muted">${escapeHtml(t("noActiveSubagents"))}</div></div>`;
    } else {
      for (const n of nodes) {
        const subs = (n.subagents || [])
          .map((s) => `<div class="muted">↳ <strong>${escapeHtml(s.role || t("subagentFallback"))}</strong> <span class="pill">${escapeHtml(s.state || "")}</span></div>`)
          .join("");
        const swarm = (n.swarm_members || [])
          .map((m) => `<div class="muted">🐝 <strong>${escapeHtml(m.name)}</strong> (${escapeHtml(m.model || "-")}) · ${escapeHtml(m.state || "-")}</div>`)
          .join("");
        const tasks = (n.tasks || [])
          .map((tk) => `<details class="cot-block"><summary>${escapeHtml(t("bgTaskPrefix"))}: ${escapeHtml(tk.task_id)}</summary><pre class="code-pre">${escapeHtml(tk.tail || "")}</pre></details>`)
          .join("");
        const shortId = (n.conversation_id || "").slice(0, 8);
        html += `<div class="card">
          <div class="card-row"><strong>${escapeHtml(n.is_root ? t("rootSession") : t("childSession"))} <span class="muted mono">#${escapeHtml(shortId)}</span></strong></div>
          ${subs}${swarm}${tasks}
        </div>`;
      }
    }
    container.innerHTML = html;
    document.getElementById("btn-refresh-tree")?.addEventListener("click", renderTreeTab);
  }

  async function renderPreviewTab() {
    const seq = ++renderSeq;
    const searchBarHtml = `<div class="card"><div class="ask-writein-row">
      <input id="preview-path-input" type="text" value="${escapeHtml(currentPreviewPath)}" placeholder="${escapeHtml(t("previewPlaceholder"))}">
      <button id="btn-preview-open" class="btn btn-primary btn-sm" type="button">${escapeHtml(t("open"))}</button>
    </div></div>`;

    if (!currentPreviewPath) {
      container.innerHTML = searchBarHtml + `<div class="card"><div class="muted">${escapeHtml(t("previewEmptyHint"))}</div></div>`;
      wirePreviewPathInput();
      return;
    }

    container.innerHTML = searchBarHtml + `<div class="inspector-loading" role="status"><span class="loading-spinner" aria-hidden="true"></span><span>${escapeHtml(t("loadingPreview"))}</span></div>`;
    wirePreviewPathInput();

    const res = await apiFetch(`/dashboard/api/file?path=${encodeURIComponent(currentPreviewPath)}&raw=0`);
    if (seq !== renderSeq || activeTab !== "preview") return;
    if (!res.ok) {
      const err = await res.json().catch(() => ({ error: `HTTP ${res.status}` }));
      if (seq !== renderSeq || activeTab !== "preview") return;
      container.innerHTML = searchBarHtml + `<div class="alert alert-danger" id="preview-error">${escapeHtml(err.error || t("previewDenied", { status: res.status }))}</div>`;
      wirePreviewPathInput();
      return;
    }
    const data = await res.json();
    if (seq !== renderSeq || activeTab !== "preview") return;
    const rawUrl = `/dashboard/api/file?path=${encodeURIComponent(data.path)}&raw=1`;
    let html = searchBarHtml + `<div class="card"><div class="card-row"><strong class="mono">${escapeHtml(data.filename)}</strong><div class="card-row">`;
    if (data.language === "markdown") {
      html += `<button id="btn-toggle-md" class="btn btn-sm" type="button">${escapeHtml(previewRawMode ? t("viewRendered") : t("viewSource"))}</button>`;
    }
    html += `<a class="btn btn-sm" href="${escapeHtml(rawUrl)}" download="${escapeHtml(data.filename)}">${escapeHtml(t("download"))}</a></div></div>`;
    html += `<div class="muted mono">${escapeHtml(data.path)} (${Number(data.size_bytes || 0).toLocaleString()} bytes)</div>`;

    if (data.language === "image" || data.language === "svg") {
      html += `<img class="preview-img" src="${escapeHtml(rawUrl)}" alt="${escapeHtml(data.filename)}">`;
    } else if (data.language === "pdf") {
      html += `<iframe class="preview-frame" src="${escapeHtml(rawUrl)}" title="PDF Preview"></iframe>`;
    } else if (data.is_binary) {
      html += `<div class="muted">${escapeHtml(t("previewBinary", { lang: data.language }))}${data.content ? `<pre class="code-pre">${escapeHtml(data.content)}</pre>` : ""}</div>`;
    } else if (data.language === "markdown" && !previewRawMode) {
      html += `<div class="msg-body">${renderMarkdown(data.content || "")}</div>`;
    } else {
      const lines = (data.content || "")
        .split("\n")
        .map((l, idx) => `<div class="code-line"><span class="ln">${idx + 1}</span><span>${escapeHtml(l)}</span></div>`)
        .join("");
      html += `${data.truncated ? `<div class="pill pill-warn">${escapeHtml(t("previewTruncated"))}</div>` : ""}<pre class="code-pre"><code>${lines}</code></pre>`;
    }
    html += `</div>`;
    container.innerHTML = html;
    wirePreviewPathInput();
    document.getElementById("btn-toggle-md")?.addEventListener("click", () => {
      previewRawMode = !previewRawMode;
      renderPreviewTab();
    });
  }

  function wirePreviewPathInput() {
    const openNow = () => {
      const v = document.getElementById("preview-path-input")?.value.trim();
      if (v) {
        currentPreviewPath = v;
        renderPreviewTab();
      }
    };
    document.getElementById("btn-preview-open")?.addEventListener("click", openNow);
    document.getElementById("preview-path-input")?.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        openNow();
      }
    });
  }

  function renderQuotaBlock(title, group) {
    if (!group) return "";
    const row = (label, lim) => {
      if (!lim) return "";
      const pct = Math.max(0, Math.min(100, Number(lim.pct || 0)));
      return `<div style="display:flex;flex-direction:column;gap:4px;">
        <div class="card-row"><span>${escapeHtml(label)}</span><span class="mono">${pct.toFixed(1)}%</span></div>
        <div class="quota-bar"><div class="quota-fill" style="width:${pct}%"></div></div>
        <div class="muted">${escapeHtml(lim.detail || "")}</div>
      </div>`;
    };
    return `<div class="card"><strong>${escapeHtml(title)}</strong>${row(t("fiveHourLimit"), group.five_hour)}${row(t("weeklyLimit"), group.weekly)}</div>`;
  }

  async function renderUsageTab() {
    const seq = ++renderSeq;
    showLoading("loadingUsage");
    const sk = getActiveSessionKey() || "";
    const res = await apiFetch(`/dashboard/api/usage?session_key=${encodeURIComponent(sk)}&all=1`);
    if (seq !== renderSeq || activeTab !== "usage") return;
    if (!res.ok) {
      container.innerHTML = `<div class="alert alert-danger">${escapeHtml(t("usageLoadError", { status: res.status }))}</div>`;
      return;
    }
    const d = await res.json();
    if (seq !== renderSeq || activeTab !== "usage") return;
    let html = `<div class="card"><div class="card-row"><strong id="usage-tab-heading">${escapeHtml(t("usageTitle"))}</strong><button id="btn-refresh-usage" class="btn btn-sm" type="button">${escapeHtml(t("refresh"))}</button></div></div>`;
    if (d.antigravity) {
      const a = d.antigravity;
      html += `<div class="card"><strong>Antigravity</strong><div class="muted">${escapeHtml(a.account || a.error || t("usageReady"))}</div></div>`;
      html += renderQuotaBlock(t("geminiGroup"), a.gemini);
      html += renderQuotaBlock(t("claudeGptGroup"), a.claude);
    }
    if (d.claude) {
      const c = d.claude;
      html += `<div class="card"><strong>Claude Code</strong><div class="muted">${escapeHtml(c.auth_label || "")} · ${escapeHtml(t("turnsLabel"))} ${Number(c.turns || 0)} · ${escapeHtml(t("inTokensLabel"))} ${Number(c.input_tokens || 0).toLocaleString()} / ${escapeHtml(t("outTokensLabel"))} ${Number(c.output_tokens || 0).toLocaleString()}</div></div>`;
      html += renderQuotaBlock(t("claudeSubGroup"), c.quota);
    }
    if (d.codex) {
      const x = d.codex;
      html += `<div class="card"><strong>OpenAI Codex</strong><div class="muted">${escapeHtml(x.auth_label || "")} · ${escapeHtml(t("tokensLabel"))} ${Number(x.total_tokens || 0).toLocaleString()} / ${Number(x.context_window || 0).toLocaleString()}</div></div>`;
      html += renderQuotaBlock(t("codexGroup"), x.quota);
    }
    container.innerHTML = html;
    document.getElementById("btn-refresh-usage")?.addEventListener("click", renderUsageTab);
  }

  async function renderCronTab() {
    const seq = ++renderSeq;
    showLoading("loadingCron");
    const res = await apiFetch("/dashboard/api/cron");
    if (seq !== renderSeq || activeTab !== "cron") return;
    if (!res.ok) {
      container.innerHTML = `<div class="alert alert-danger">${escapeHtml(t("cronLoadError", { status: res.status }))}</div>`;
      return;
    }
    const data = await res.json();
    if (seq !== renderSeq || activeTab !== "cron") return;
    const jobs = data.jobs || [];
    let html = `<div class="card"><div class="card-row"><strong id="cron-tab-heading">${escapeHtml(t("cronTitle", { count: jobs.length }))}</strong><button id="btn-refresh-cron" class="btn btn-sm" type="button">${escapeHtml(t("refresh"))}</button></div></div>`;
    if (jobs.length === 0) {
      html += `<div class="card"><div class="muted">${escapeHtml(t("cronEmpty"))}</div></div>`;
    }
    for (const j of jobs) {
      const badge = j.enabled
        ? `<span class="pill pill-ok">${escapeHtml(t("cronEnabled"))}</span>`
        : `<span class="pill">${escapeHtml(t("cronPaused"))}</span>`;
      html += `<div class="card">
        <div class="card-row"><strong>${escapeHtml(j.title)}</strong>${badge}</div>
        <div class="muted mono">${escapeHtml(j.schedule)} (${escapeHtml(j.timezone || "UTC")})</div>
        <div class="muted">${escapeHtml(t("cronNext"))}: ${escapeHtml(j.next_run || "-")} · ${escapeHtml(t("cronLast"))}: ${escapeHtml(j.last_run_status || t("cronNever"))}</div>
        <div class="card-row">
          <button class="btn btn-sm cron-toggle-btn" data-id="${escapeHtml(j.id)}" type="button">${escapeHtml(j.enabled ? t("cronPauseBtn") : t("cronEnableBtn"))}</button>
          <button class="btn btn-primary btn-sm cron-run-btn" data-id="${escapeHtml(j.id)}" type="button">${escapeHtml(t("cronRunNowBtn"))}</button>
        </div>
      </div>`;
    }
    container.innerHTML = html;
    document.getElementById("btn-refresh-cron")?.addEventListener("click", renderCronTab);
    container.querySelectorAll(".cron-toggle-btn").forEach((b) => {
      b.addEventListener("click", async () => {
        await apiFetch(`/dashboard/api/cron/${encodeURIComponent(b.dataset.id)}/toggle`, { method: "POST" });
        renderCronTab();
      });
    });
    container.querySelectorAll(".cron-run-btn").forEach((b) => {
      b.addEventListener("click", async () => {
        b.disabled = true;
        const sk = typeof getActiveSessionKey === "function" ? getActiveSessionKey() : "";
        await apiFetch(`/dashboard/api/cron/${encodeURIComponent(b.dataset.id)}/run`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ session_key: sk || null })
        });
        b.textContent = t("cronTriggeredBtn");
      });
    });
  }

  async function renderVaultTab() {
    const seq = ++renderSeq;
    showLoading("loadingVault");
    const [vRes, hRes] = await Promise.all([
      apiFetch("/dashboard/api/vault/entries"),
      apiFetch("/dashboard/api/hitl")
    ]);
    if (seq !== renderSeq || activeTab !== "vault") return;
    if (!vRes.ok) {
      container.innerHTML = `<div class="alert alert-danger">${escapeHtml(t("vaultLoadError", { status: vRes.status }))}</div>`;
      return;
    }
    const vData = await vRes.json();
    const hData = hRes.ok ? await hRes.json() : { active: false };
    if (seq !== renderSeq || activeTab !== "vault") return;
    let html = `<div class="card">
      <div class="card-row"><strong>${escapeHtml(t("vaultTitle", { profile: vData.profile || "" }))}</strong>
      <span class="pill ${hData.active ? "pill-warn" : ""}">${escapeHtml(hData.active ? t("hitlActive") : t("hitlIdle"))}</span></div>
      <div class="muted">${escapeHtml(t("vaultZeroKnowledgeNote"))}</div>
    </div>
    <form id="vault-upsert-form" class="card">
      <strong>${escapeHtml(t("vaultFormTitle"))}</strong>
      <input id="v-site-key" type="text" placeholder="${escapeHtml(t("vaultKeyPh"))}" required>
      <input id="v-title" type="text" placeholder="${escapeHtml(t("vaultNamePh"))}">
      <input id="v-url" type="url" placeholder="https://...">
      <input id="v-user" type="text" placeholder="${escapeHtml(t("vaultUserPh"))}" required>
      <input id="v-pass" type="password" placeholder="${escapeHtml(t("vaultPassPh"))}">
      <button class="btn btn-primary btn-sm" type="submit">${escapeHtml(t("vaultSaveBtn"))}</button>
    </form>`;

    for (const e of vData.entries || []) {
      html += `<div class="card">
        <div class="card-row"><strong class="mono">${escapeHtml(e.site_key)}</strong>
          <div class="card-row">
            <button class="btn btn-sm vault-auth-btn" data-key="${escapeHtml(e.site_key)}" type="button">${escapeHtml(t("vaultAuthBtn"))}</button>
            <button class="btn btn-danger btn-sm vault-del-btn" data-key="${escapeHtml(e.site_key)}" type="button">${escapeHtml(t("vaultDeleteBtn"))}</button>
          </div>
        </div>
        <div class="muted">${escapeHtml(e.title)} · ${escapeHtml(t("vaultAccountLabel"))}: <code>${escapeHtml(e.masked_username)}</code></div>
      </div>`;
    }
    container.innerHTML = html;

    document.getElementById("vault-upsert-form")?.addEventListener("submit", async (ev) => {
      ev.preventDefault();
      await apiFetch("/dashboard/api/vault/entries", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          site_key: document.getElementById("v-site-key").value.trim(),
          title: document.getElementById("v-title").value.trim(),
          url: document.getElementById("v-url").value.trim(),
          username: document.getElementById("v-user").value.trim(),
          password: document.getElementById("v-pass").value
        })
      });
      renderVaultTab();
    });
    container.querySelectorAll(".vault-auth-btn").forEach((b) => {
      b.addEventListener("click", () => launchBrowserAuth(b.dataset.key));
    });
    container.querySelectorAll(".vault-del-btn").forEach((b) => {
      b.addEventListener("click", async () => {
        await apiFetch(`/dashboard/api/vault/entries/${encodeURIComponent(b.dataset.key)}`, { method: "DELETE" });
        renderVaultTab();
      });
    });
  }

  async function launchBrowserAuth(targetValue, explicitUrl = "") {
    const cleanTarget = (targetValue || "").trim();
    if (!cleanTarget) return;
    lastDoneSiteKey = "";
    openPanel();
    activeTab = "browser";
    inspectorEl.classList.add("wide-browser");
    tabsNav.querySelectorAll(".tab-btn").forEach((btn) => {
      btn.classList.toggle("active", btn.dataset.tab === "browser");
    });
    onViewChange?.();
    const seq = ++renderSeq;
    showLoading("launchingBrowser");
    const res = await apiFetch("/dashboard/api/hitl/start", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ target: cleanTarget, url: explicitUrl || null })
    });
    if (seq !== renderSeq || activeTab !== "browser") return;
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      await renderBrowserTab(false, err.error || `${t("browserLaunchErr")} (${res.status})`);
      return;
    }
    const data = await res.json();
    if (data.session?.token) {
      activeHitlToken = data.session.token;
    }
    await renderBrowserTab(false);
  }

  async function renderBrowserTab(preserveMounted = false, errMessage = "") {
    if (preserveMounted && activeTab === "browser") {
      const mounted = document.getElementById("hitl-iframe");
      if (mounted && mounted.dataset.token) {
        const check = await apiFetch("/dashboard/api/hitl");
        if (check.ok) {
          const d = await check.json();
          if ((d.sessions || []).some((s) => s.token === mounted.dataset.token)) {
            return;
          }
        }
      }
    }
    const seq = ++renderSeq;
    showLoading("loadingBrowser");
    const res = await apiFetch("/dashboard/api/hitl");
    if (seq !== renderSeq || activeTab !== "browser") return;
    const data = res.ok ? await res.json() : { sessions: [] };
    if (seq !== renderSeq || activeTab !== "browser") return;
    const list = data.sessions || [];

    if (list.length > 0 && !list.some((s) => s.token === activeHitlToken)) {
      activeHitlToken = list[0].token;
    } else if (list.length === 0) {
      activeHitlToken = "";
    }
    const chosen = list.find((s) => s.token === activeHitlToken) || null;

    let html = "";
    if (errMessage) {
      html += `<div class="alert alert-danger" id="hitl-launch-error">${escapeHtml(errMessage)}</div>`;
    }
    if (lastDoneSiteKey) {
      html += `<div class="alert alert-ok" id="hitl-done-alert">${escapeHtml(t("browserDoneNotice", { site: lastDoneSiteKey }))}</div>`;
    }

    if (chosen) {
      const viewerUrl = chosen.viewer_path || `/auth/${chosen.token}`;
      const mins = Math.max(1, Math.ceil(Number(chosen.remaining_secs || 0) / 60));
      const switcher = list.length > 1
        ? `<select id="hitl-session-select" class="ctrl-select">${list.map((s) => `<option value="${escapeHtml(s.token)}"${s.token === chosen.token ? " selected" : ""}>${escapeHtml(s.site_key || "auth")}</option>`).join("")}</select>`
        : `<strong>🌐 ${escapeHtml(chosen.site_key || "auth")}</strong> <span class="pill pill-ok">~${mins}m</span>`;
      html += `<div class="card" style="padding:8px 11px;">
        <div class="card-row">
          <div class="card-row">${switcher}</div>
          <div class="card-row">
            <a id="hitl-popout-btn" class="btn btn-sm" href="${escapeHtml(viewerUrl)}" target="_blank" rel="noopener">${escapeHtml(t("browserNewTabBtn"))} ↗</a>
            <button id="btn-refresh-browser" class="btn btn-sm" type="button">${escapeHtml(t("refresh"))}</button>
          </div>
        </div>
      </div>
      <div class="hitl-frame-wrap">
        <iframe id="hitl-iframe" class="hitl-frame" data-token="${escapeHtml(chosen.token)}" src="${escapeHtml(viewerUrl)}" title="Remote Browser"></iframe>
      </div>`;
    } else {
      html += `<form id="hitl-launch-form" class="card">
        <div class="card-row"><strong id="browser-tab-heading">${escapeHtml(t("browserLaunchTitle"))}</strong>
          <button id="btn-refresh-browser" class="btn btn-sm" type="button">${escapeHtml(t("refresh"))}</button>
        </div>
        <div class="muted">${escapeHtml(t("browserLaunchHint"))}</div>
        <div class="ask-writein-row">
          <input id="hitl-target-input" type="text" placeholder="${escapeHtml(t("browserTargetPh"))}" required>
          <button id="btn-hitl-launch" class="btn btn-primary btn-sm" type="submit">${escapeHtml(t("browserLaunchBtn"))}</button>
        </div>
      </form>`;
    }

    container.innerHTML = html;
    document.getElementById("btn-refresh-browser")?.addEventListener("click", () => renderBrowserTab(false));
    document.getElementById("hitl-session-select")?.addEventListener("change", (e) => {
      activeHitlToken = e.target.value;
      renderBrowserTab(false);
    });
    document.getElementById("hitl-launch-form")?.addEventListener("submit", (e) => {
      e.preventDefault();
      const val = document.getElementById("hitl-target-input")?.value || "";
      launchBrowserAuth(val);
    });
  }

  function openBrowserSession(token = "") {
    if (token) activeHitlToken = token;
    lastDoneSiteKey = "";
    openTab("browser");
  }

  function notifyBrowserDone(siteKey) {
    lastDoneSiteKey = siteKey || "site";
    activeHitlToken = "";
    if (!isCollapsed() && activeTab === "browser") {
      renderBrowserTab(false);
    }
  }

  async function renderSecurityTab() {
    const seq = ++renderSeq;
    showLoading("loadingSecurity");
    const res = await apiFetch("/dashboard/api/auth/me");
    if (seq !== renderSeq || activeTab !== "security") return;
    const me = res.ok ? await res.json() : {};
    if (seq !== renderSeq || activeTab !== "security") return;
    container.innerHTML = `
      <div class="card">
        <strong>${escapeHtml(t("secStatusTitle"))}</strong>
        <div class="muted">${escapeHtml(t("secProfileLabel"))}: <code>${escapeHtml(me.profile || "default")}</code> · ${escapeHtml(t("secPinConfigured"))}: <strong>${escapeHtml(me.has_pin ? t("yes") : t("no"))}</strong></div>
      </div>
      <form id="sec-pin-form" class="card">
        <strong>${escapeHtml(t("secPinFormTitle"))}</strong>
        <input id="sec-pin-input" type="password" minlength="4" placeholder="${escapeHtml(t("secPinPh"))}" required>
        <button class="btn btn-primary btn-sm" type="submit">${escapeHtml(t("secPinSaveBtn"))}</button>
        <div id="sec-pin-msg" class="hidden"></div>
      </form>
      <div class="card">
        <strong>${escapeHtml(t("secRevokeTitle"))}</strong>
        <div class="card-row">
          <button id="btn-logout" class="btn btn-sm" type="button">${escapeHtml(t("secLogoutBtn"))}</button>
          <button id="btn-revoke-all" class="btn btn-danger btn-sm" type="button">${escapeHtml(t("secRevokeAllBtn"))}</button>
        </div>
      </div>`;

    document.getElementById("sec-pin-form")?.addEventListener("submit", async (ev) => {
      ev.preventDefault();
      const pin = document.getElementById("sec-pin-input").value;
      const r = await apiFetch("/dashboard/api/auth/pin", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ pin })
      });
      const msgEl = document.getElementById("sec-pin-msg");
      if (r.ok) {
        msgEl.className = "alert alert-ok";
        msgEl.textContent = t("secPinSavedMsg");
        document.getElementById("sec-pin-input").value = "";
      } else {
        const d = await r.json().catch(() => ({}));
        msgEl.className = "alert alert-danger";
        msgEl.textContent = d.error || t("secPinFailMsg");
      }
      msgEl.classList.remove("hidden");
    });

    document.getElementById("btn-logout")?.addEventListener("click", async () => {
      await apiFetch("/dashboard/api/auth/logout", { method: "POST" });
      onForceLogout();
    });
    document.getElementById("btn-revoke-all")?.addEventListener("click", async () => {
      await apiFetch("/dashboard/api/auth/revoke_all", { method: "POST" });
      onForceLogout();
    });
  }

  const enginesRenderer = createOnboardingRenderer({
    apiFetch,
    onOpenBrowserToken: (tok) => openBrowserSession(tok)
  });

  async function renderEnginesTab() {
    const seq = ++renderSeq;
    showLoading("loadingEngines");
    if (seq !== renderSeq || activeTab !== "engines") return;
    await enginesRenderer.renderInto(container, { isWizard: false });
  }

  function refreshCurrentTab() {
    if (activeTab !== "engines") enginesRenderer.clearPoll();
    if (activeTab === "tree") return renderTreeTab();
    if (activeTab === "preview") return renderPreviewTab();
    if (activeTab === "browser") return renderBrowserTab(false);
    if (activeTab === "engines") return renderEnginesTab();
    if (activeTab === "usage") return renderUsageTab();
    if (activeTab === "cron") return renderCronTab();
    if (activeTab === "vault") return renderVaultTab();
    if (activeTab === "security") return renderSecurityTab();
  }

  tabsNav.querySelectorAll(".tab-btn").forEach((btn) => {
    btn.addEventListener("click", () => setTab(btn.dataset.tab));
  });

  onLangChange((nextLang) => {
    if (!isCollapsed()) {
      const hitlFrame = document.getElementById("hitl-iframe");
      if (activeTab === "browser" && hitlFrame) {
        const popBtn = document.getElementById("hitl-popout-btn");
        if (popBtn) popBtn.textContent = `${t("browserNewTabBtn")} ↗`;
        const refBtn = document.getElementById("btn-refresh-browser");
        if (refBtn) refBtn.textContent = t("refresh");
        try {
          hitlFrame.contentWindow?.postMessage({ type: "tuner_set_lang", lang: nextLang || getLang() }, window.location.origin);
        } catch (_) {}
        return;
      }
      refreshCurrentTab();
    }
  });

  return {
    setTab,
    openTab,
    togglePanel,
    closePanel,
    isCollapsed,
    refreshCurrentTab,
    openFilePreview,
    openBrowserSession,
    launchBrowserAuth,
    notifyBrowserDone,
    onHitlUpdated: () => {
      if (!isCollapsed() && activeTab === "browser") {
        renderBrowserTab(true);
      }
    },
    onProviderSetupUpdated: async () => {
      if (!isCollapsed() && activeTab === "engines") {
        renderEnginesTab();
      } else if (!isCollapsed() && activeTab === "browser") {
        const r = await apiFetch("/dashboard/api/hitl").catch(() => null);
        if (r && r.ok) {
          const d = await r.json().catch(() => ({}));
          if (!d.sessions || d.sessions.length === 0) closePanel();
        }
      }
    },
    getActiveTab: () => activeTab
  };
}
