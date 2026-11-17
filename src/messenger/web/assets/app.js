import { escapeHtml, base64ToUtf8 } from "/dashboard/assets/markdown.js";
import { initLang, toggleLang, applyDomI18n, onLangChange, t } from "/dashboard/assets/i18n.js";
import { createChatController } from "/dashboard/assets/chat.js";
import { createInspectorController } from "/dashboard/assets/inspector.js";

let csrfToken = "";
let sessions = [];
let ws = null;
let wsRetryMs = 1000;

async function apiFetch(url, options = {}) {
  const opts = { credentials: "same-origin", ...options };
  const method = (opts.method || "GET").toUpperCase();
  const headers = new Headers(opts.headers || {});
  if (method !== "GET" && method !== "HEAD" && csrfToken) {
    headers.set("x-tuner-csrf", csrfToken);
  }
  opts.headers = headers;
  const res = await fetch(url, opts);
  if (res.status === 401) {
    window.location.replace("/dashboard/login");
  }
  return res;
}

function redirectToLogin() {
  window.location.replace("/dashboard/login");
}

const inspector = createInspectorController({
  apiFetch,
  getActiveSessionKey: () => chat.getActiveSessionKey(),
  onForceLogout: redirectToLogin
});

const chat = createChatController({
  apiFetch,
  onOpenFile: (path) => inspector.openFilePreview(path),
  onOpenInspectorTab: (tab) => inspector.openTab(tab),
  onRefreshSessions: () => loadSessions(false)
});

function closeDrawers() {
  document.getElementById("sidebar").classList.remove("open");
  document.getElementById("inspector").classList.remove("open");
  document.getElementById("drawer-backdrop").classList.add("hidden");
}

function formatShortModel(model) {
  if (!model) return "";
  return model.replace(/^antigravity-/, "");
}

function renderSessionList() {
  const listEl = document.getElementById("session-list");
  const query = (document.getElementById("session-search").value || "").trim().toLowerCase();
  const activeKey = chat.getActiveSessionKey();
  listEl.innerHTML = "";

  const filtered = sessions.filter((s) => {
    if (!query) return true;
    const hay = `${s.topic_name || ""} ${s.session_key} ${s.provider} ${s.model}`.toLowerCase();
    return hay.includes(query);
  });

  if (filtered.length === 0) {
    listEl.innerHTML = `<div class="muted" style="padding:12px 10px;">${escapeHtml(t("noMatchingSessions"))}</div>`;
    return;
  }

  for (const s of filtered) {
    const row = document.createElement("div");
    row.className = `session-row${s.session_key === activeKey ? " active" : ""}`;

    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "session-item";
    btn.dataset.sessionKey = s.session_key;

    const label = s.topic_name || (s.transport === "web"
      ? t("webTopicFallback", { id: s.topic_id })
      : t("tgTopicFallback", { id: s.topic_id || s.chat_id }));
    const statusDot = s.busy || s.asking ? `<span class="pulse-dot" title="${escapeHtml(t("runningTitle"))}"></span>` : "";
    const tgBadge = s.transport === "tg" ? `<span class="tg-tag" title="${escapeHtml(t("tgMirroredTitle"))}">TG</span>` : "";

    btn.innerHTML = `
      <div class="session-item-top">
        <span class="session-name">${escapeHtml(label)}</span>
        <span style="display:flex;align-items:center;gap:5px;">${statusDot}${tgBadge}</span>
      </div>
      <div class="session-item-meta">
        <span>${escapeHtml(s.provider)}</span>
        <span>·</span>
        <span>${escapeHtml(formatShortModel(s.model))}</span>
      </div>`;

    btn.addEventListener("click", () => {
      closeDrawers();
      chat.selectSession(s);
      renderSessionList();
      if (!inspector.isCollapsed() && (inspector.getActiveTab() === "tree" || inspector.getActiveTab() === "usage")) {
        inspector.refreshCurrentTab();
      }
    });
    row.appendChild(btn);

    if (s.transport === "web") {
      const delBtn = document.createElement("button");
      delBtn.type = "button";
      delBtn.className = "session-del-btn";
      delBtn.title = t("deleteTopicTitle");
      delBtn.textContent = "×";
      delBtn.addEventListener("click", async (ev) => {
        ev.stopPropagation();
        const res = await apiFetch("/dashboard/api/sessions", {
          method: "DELETE",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ session_key: s.session_key })
        });
        if (res.ok) {
          const wasActive = s.session_key === chat.getActiveSessionKey();
          await loadSessions(wasActive);
          if (wasActive && sessions.length > 0) {
            await chat.selectSession(sessions[0]);
            renderSessionList();
          } else if (sessions.length === 0) {
            await chat.selectSession(null);
          }
        }
      });
      row.appendChild(delBtn);
    }

    listEl.appendChild(row);
  }
}

async function loadSessions(selectFirstIfNone = true) {
  const res = await apiFetch("/dashboard/api/sessions");
  if (!res.ok) return;
  const data = await res.json();
  sessions = data.sessions || [];
  const wp = data.warm_pools || {};
  const wpEl = document.getElementById("warm-pool-pill");
  if (wpEl) wpEl.textContent = `${Number(wp.total_active || 0)}`;

  const activeKey = chat.getActiveSessionKey();
  if (selectFirstIfNone && (!activeKey || !sessions.some((s) => s.session_key === activeKey)) && sessions.length > 0) {
    await chat.selectSession(sessions[0]);
  } else if (activeKey) {
    const fresh = sessions.find((s) => s.session_key === activeKey);
    if (fresh) {
      chat.handleWsEvent({
        type: "session_updated",
        session_key: fresh.session_key,
        provider: fresh.provider,
        model: fresh.model,
        effort: fresh.effort,
        topic_name: fresh.topic_name,
        busy: fresh.busy,
        asking: fresh.asking
      });
    }
  }
  renderSessionList();
}

function connectWebSocket() {
  const dot = document.getElementById("ws-status");
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const wsUrl = `${proto}//${window.location.host}/dashboard/api/ws?csrf=${encodeURIComponent(csrfToken)}`;
  ws = new WebSocket(wsUrl);

  ws.addEventListener("open", () => {
    wsRetryMs = 1000;
    dot.className = "ws-dot online";
    dot.title = t("wsConnected");
  });

  ws.addEventListener("message", (msgEv) => {
    try {
      const ev = JSON.parse(msgEv.data);
      chat.handleWsEvent(ev);
      if (ev.type === "session_updated") {
        const target = sessions.find((s) => s.session_key === ev.session_key);
        if (target) {
          target.provider = ev.provider;
          target.model = ev.model;
          target.effort = ev.effort;
          target.topic_name = ev.topic_name;
          target.busy = ev.busy;
          target.asking = ev.asking;
          renderSessionList();
        } else {
          loadSessions(false);
        }
      } else if (ev.type === "turn_result") {
        loadSessions(false);
        if (!inspector.isCollapsed() && inspector.getActiveTab() === "tree") {
          inspector.refreshCurrentTab();
        }
      } else if (ev.type === "cron_updated" && !inspector.isCollapsed() && inspector.getActiveTab() === "cron") {
        inspector.refreshCurrentTab();
      }
    } catch (_) {}
  });

  ws.addEventListener("close", () => {
    dot.className = "ws-dot offline";
    dot.title = t("wsReconnecting");
    setTimeout(connectWebSocket, wsRetryMs);
    wsRetryMs = Math.min(wsRetryMs * 2, 15000);
  });
}

function syncThemeIcon() {
  const btn = document.getElementById("btn-theme");
  if (!btn) return;
  const attr = document.documentElement.getAttribute("data-theme");
  const isDark = attr ? attr === "dark" : window.matchMedia?.("(prefers-color-scheme: dark)").matches;
  btn.textContent = isDark ? "☀" : "☾";
  btn.title = isDark ? t("switchToLight") : t("switchToDark");
}

function setupUiListeners() {
  document.getElementById("session-search").addEventListener("input", renderSessionList);

  const newForm = document.getElementById("new-topic-form");
  document.getElementById("btn-new-topic").addEventListener("click", () => {
    newForm.classList.toggle("hidden");
    if (!newForm.classList.contains("hidden")) {
      document.getElementById("new-topic-name").focus();
    }
  });
  document.getElementById("btn-create-topic-cancel").addEventListener("click", () => {
    newForm.classList.add("hidden");
  });
  newForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    const nameEl = document.getElementById("new-topic-name");
    const provEl = document.getElementById("new-topic-provider");
    const res = await apiFetch("/dashboard/api/sessions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        topic_name: nameEl.value.trim() || null,
        provider: provEl.value
      })
    });
    if (res.ok) {
      const created = await res.json();
      nameEl.value = "";
      newForm.classList.add("hidden");
      await loadSessions(false);
      await chat.selectSession(created);
      renderSessionList();
      closeDrawers();
    }
  });

  document.getElementById("btn-toggle-sidebar").addEventListener("click", () => {
    const sb = document.getElementById("sidebar");
    const open = sb.classList.toggle("open");
    document.getElementById("inspector").classList.remove("open");
    document.getElementById("drawer-backdrop").classList.toggle("hidden", !open);
  });

  document.getElementById("btn-toggle-inspector").addEventListener("click", () => {
    inspector.togglePanel();
  });

  document.getElementById("btn-close-inspector")?.addEventListener("click", () => {
    inspector.closePanel();
  });

  document.getElementById("drawer-backdrop").addEventListener("click", closeDrawers);

  document.getElementById("btn-lang")?.addEventListener("click", () => {
    toggleLang();
  });

  onLangChange(() => {
    renderSessionList();
    syncThemeIcon();
    const dot = document.getElementById("ws-status");
    if (dot) {
      dot.title = dot.classList.contains("online") ? t("wsConnected") : t("wsReconnecting");
    }
  });

  document.getElementById("btn-theme")?.addEventListener("click", () => {
    const attr = document.documentElement.getAttribute("data-theme");
    const currentlyDark = attr ? attr === "dark" : window.matchMedia?.("(prefers-color-scheme: dark)").matches;
    const next = currentlyDark ? "light" : "dark";
    document.documentElement.setAttribute("data-theme", next);
    try { localStorage.setItem("tuner_theme", next); } catch (_) {}
    syncThemeIcon();
  });
  syncThemeIcon();

  document.getElementById("app-shell").addEventListener("click", (e) => {
    const copyBtn = e.target.closest(".copy-code-btn");
    if (copyBtn) {
      try {
        const raw = base64ToUtf8(copyBtn.dataset.codeB64 || "");
        navigator.clipboard?.writeText(raw);
        copyBtn.textContent = t("copied");
        setTimeout(() => { copyBtn.textContent = t("copy"); }, 1200);
      } catch (_) {}
      return;
    }
    const chip = e.target.closest(".file-chip");
    if (chip && chip.dataset.filePath) {
      inspector.openFilePreview(chip.dataset.filePath);
    }
  });
}

async function boot() {
  applyDomI18n();
  const meRes = await fetch("/dashboard/api/auth/me", { credentials: "same-origin" });
  if (!meRes.ok) {
    redirectToLogin();
    return;
  }
  const me = await meRes.json();
  if (!me.authenticated) {
    redirectToLogin();
    return;
  }
  csrfToken = me.csrf_token || "";
  initLang(me.language);
  applyDomI18n();

  const profEl = document.getElementById("profile-badge");
  if (profEl) {
    profEl.textContent = me.profile || "default";
    profEl.classList.toggle("hidden", !me.profile || me.profile === "default");
  }

  setupUiListeners();
  await loadSessions(true);
  connectWebSocket();

  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.register("/dashboard/sw.js", { scope: "/dashboard" }).catch(() => {});
  }
}

boot();
