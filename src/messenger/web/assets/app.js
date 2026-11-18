import { escapeHtml, base64ToUtf8 } from "/dashboard/assets/markdown.js";
import { initLang, toggleLang, applyDomI18n, onLangChange, t } from "/dashboard/assets/i18n.js";
import { createChatController } from "/dashboard/assets/chat.js";
import { createInspectorController } from "/dashboard/assets/inspector.js";

let csrfToken = "";
let sessions = [];
let activeTransport = "web";
let telegramConnected = true;
let telegramBotUsername = "";
let renamingKey = "";
let confirmingDeleteKey = "";
let confirmDeleteTimer = null;
let activeHitlSessions = [];
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

function getPreferredTgChatId() {
  const activeSk = chat.getActiveSessionKey();
  const activeObj = sessions.find((s) => s.session_key === activeSk && s.transport === "tg" && s.chat_id !== 0);
  if (activeObj) return activeObj.chat_id;
  const recentTg = sessions.find((s) => s.transport === "tg" && s.chat_id !== 0 && s.topic_id != null);
  return recentTg ? recentTg.chat_id : null;
}

const inspector = createInspectorController({
  apiFetch,
  getActiveSessionKey: () => chat.getActiveSessionKey(),
  onForceLogout: redirectToLogin,
  onViewChange: () => renderHitlBannerDom()
});

const chat = createChatController({
  apiFetch,
  onOpenFile: (path) => inspector.openFilePreview(path),
  onOpenInspectorTab: (tab) => inspector.openTab(tab),
  onOpenBrowserToken: (tok) => inspector.openBrowserSession(tok),
  onRefreshSessions: () => loadSessions(),
  onTelegramConnected: (uname) => {
    telegramConnected = true;
    telegramBotUsername = uname || "";
    renderSessionList();
  },
  getActiveTransport: () => activeTransport,
  getPreferredTgChatId
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

function getSessionLabel(s) {
  if (s.topic_name) return s.topic_name;
  return s.transport === "web"
    ? t("webTopicFallback", { id: s.topic_id })
    : t("tgTopicFallback", { id: s.topic_id || s.chat_id });
}

function bucketSession(s, now) {
  if (s.pinned) return "pinned";
  const ts = Date.parse(s.last_active || s.last_modified || "");
  if (!Number.isFinite(ts)) return "older";
  const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  if (ts >= todayStart) return "today";
  if (ts >= todayStart - 86_400_000) return "yesterday";
  if (ts >= todayStart - 7 * 86_400_000) return "prev7";
  if (ts >= todayStart - 30 * 86_400_000) return "prev30";
  return "older";
}

const BUCKET_ORDER = [
  ["pinned", "grpPinned"],
  ["today", "grpToday"],
  ["yesterday", "grpYesterday"],
  ["prev7", "grpPrev7"],
  ["prev30", "grpPrev30"],
  ["older", "grpOlder"]
];

function syncTransportTabs() {
  const isTg = activeTransport === "tg";
  const isDraft = !chat.getActiveSessionKey();
  const webCnt = sessions.filter((s) => s.transport === "web").length;
  const tgCnt = sessions.filter((s) => s.transport === "tg").length;
  const webCntEl = document.getElementById("seg-web-count");
  const tgCntEl = document.getElementById("seg-tg-count");
  if (webCntEl) webCntEl.textContent = `${webCnt}`;
  if (tgCntEl) tgCntEl.textContent = `${tgCnt}`;
  document.getElementById("seg-web")?.classList.toggle("active", !isTg);
  document.getElementById("seg-tg")?.classList.toggle("active", isTg);

  const newBtn = document.getElementById("btn-new-topic");
  if (newBtn) {
    newBtn.textContent = isTg ? t("newTgTopicBtn") : t("newTopicBtn");
    newBtn.title = isTg ? t("newTgTopicTitle") : t("newTopicTitle");
    newBtn.classList.toggle("active", isDraft);
    newBtn.classList.toggle("hidden", isTg && !telegramConnected);
  }
}

async function patchSessionMeta(sessionKey, patchBody) {
  const res = await apiFetch("/dashboard/api/sessions/config", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ session_key: sessionKey, ...patchBody })
  });
  if (res.ok) {
    await loadSessions();
  }
}

function buildSessionRow(s, activeKey) {
  const row = document.createElement("div");
  row.className = `session-row${s.session_key === activeKey ? " active" : ""}`;

  if (renamingKey === s.session_key) {
    const inp = document.createElement("input");
    inp.type = "text";
    inp.className = "rename-input";
    inp.maxLength = 80;
    inp.value = getSessionLabel(s);
    let committed = false;
    const commit = async (save) => {
      if (committed) return;
      committed = true;
      const nextVal = inp.value.trim();
      renamingKey = "";
      if (save && nextVal && nextVal !== s.topic_name) {
        await patchSessionMeta(s.session_key, { topic_name: nextVal });
      } else {
        renderSessionList();
      }
    };
    inp.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        commit(true);
      } else if (e.key === "Escape") {
        e.preventDefault();
        commit(false);
      }
    });
    inp.addEventListener("blur", () => commit(true));
    row.appendChild(inp);
    setTimeout(() => {
      inp.focus();
      inp.select();
    }, 0);
    return row;
  }

  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "session-item";
  btn.dataset.sessionKey = s.session_key;

  const label = getSessionLabel(s);
  const statusDot = s.busy || s.asking ? `<span class="pulse-dot" title="${escapeHtml(t("runningTitle"))}"></span>` : "";

  btn.innerHTML = `
    <div class="session-item-top">
      <span class="session-name">${escapeHtml(label)}</span>
      ${statusDot}
    </div>
    <div class="session-item-meta">
      <span>${escapeHtml(s.provider)}</span>
      <span>·</span>
      <span class="session-model-text">${escapeHtml(formatShortModel(s.model))}</span>
    </div>`;

  btn.addEventListener("click", async () => {
    confirmingDeleteKey = "";
    closeDrawers();
    await chat.selectSession(s);
    renderSessionList();
    if (!inspector.isCollapsed() && (inspector.getActiveTab() === "tree" || inspector.getActiveTab() === "usage")) {
      inspector.refreshCurrentTab();
    }
  });
  row.appendChild(btn);

  const actions = document.createElement("div");
  actions.className = "session-actions";

  const pinBtn = document.createElement("button");
  pinBtn.type = "button";
  pinBtn.className = `session-act-btn session-pin-btn${s.pinned ? " pinned" : ""}`;
  pinBtn.title = s.pinned ? t("unpinTopicTitle") : t("pinTopicTitle");
  pinBtn.textContent = "📌";
  pinBtn.addEventListener("click", async (ev) => {
    ev.stopPropagation();
    await patchSessionMeta(s.session_key, { pinned: !s.pinned });
  });
  actions.appendChild(pinBtn);

  const renBtn = document.createElement("button");
  renBtn.type = "button";
  renBtn.className = "session-act-btn session-rename-btn";
  renBtn.title = t("renameTopicTitle");
  renBtn.textContent = "✎";
  renBtn.addEventListener("click", (ev) => {
    ev.stopPropagation();
    confirmingDeleteKey = "";
    renamingKey = s.session_key;
    renderSessionList();
  });
  actions.appendChild(renBtn);

  const isConfirmingTgDel = s.transport === "tg" && confirmingDeleteKey === s.session_key;
  const delBtn = document.createElement("button");
  delBtn.type = "button";
  delBtn.className = `session-del-btn${isConfirmingTgDel ? " confirm-del" : ""}`;
  delBtn.title = s.transport === "tg" ? t("deleteTgTopicTitle") : t("deleteTopicTitle");
  delBtn.textContent = isConfirmingTgDel ? t("confirmDeleteTgBtn") : "×";
  delBtn.addEventListener("click", async (ev) => {
    ev.stopPropagation();
    if (s.transport === "tg" && confirmingDeleteKey !== s.session_key) {
      confirmingDeleteKey = s.session_key;
      if (confirmDeleteTimer) clearTimeout(confirmDeleteTimer);
      confirmDeleteTimer = setTimeout(() => {
        if (confirmingDeleteKey === s.session_key) {
          confirmingDeleteKey = "";
          renderSessionList();
        }
      }, 3500);
      renderSessionList();
      return;
    }
    confirmingDeleteKey = "";
    const res = await apiFetch("/dashboard/api/sessions", {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_key: s.session_key,
        delete_remote_topic: s.transport === "tg"
      })
    });
    if (res.ok) {
      const wasActive = s.session_key === chat.getActiveSessionKey();
      await loadSessions();
      if (wasActive) {
        await chat.selectSession(null);
        renderSessionList();
      }
    }
  });
  actions.appendChild(delBtn);

  row.appendChild(actions);
  return row;
}

function renderSessionList() {
  syncTransportTabs();
  const listEl = document.getElementById("session-list");
  const query = (document.getElementById("session-search").value || "").trim().toLowerCase();
  const activeKey = chat.getActiveSessionKey();
  listEl.innerHTML = "";

  if (activeTransport === "tg") {
    const hint = document.createElement("div");
    hint.className = "sidebar-hint";
    hint.textContent = telegramConnected ? t("tgTabHint") : t("tgDisconnectedHint");
    listEl.appendChild(hint);
  }

  const tabSessions = sessions.filter((s) => s.transport === activeTransport);
  const filtered = tabSessions.filter((s) => {
    if (!query) return true;
    const hay = `${s.topic_name || ""} ${s.session_key} ${s.provider} ${s.model}`.toLowerCase();
    return hay.includes(query);
  });

  if (filtered.length === 0) {
    const msgKey = !query
      ? (activeTransport === "tg" ? "emptyNoTgSessions" : "emptyNoWebSessions")
      : "noMatchingSessions";
    const empty = document.createElement("div");
    empty.className = "muted";
    empty.style.padding = "12px 10px";
    empty.textContent = t(msgKey);
    listEl.appendChild(empty);
    return;
  }

  const now = new Date();
  const buckets = new Map();
  for (const s of filtered) {
    const b = bucketSession(s, now);
    if (!buckets.has(b)) buckets.set(b, []);
    buckets.get(b).push(s);
  }

  for (const [bKey, i18nKey] of BUCKET_ORDER) {
    const group = buckets.get(bKey);
    if (!group || group.length === 0) continue;
    const hdr = document.createElement("div");
    hdr.className = "session-group-label";
    hdr.dataset.bucket = bKey;
    hdr.textContent = bKey === "pinned" ? `📌 ${t(i18nKey)}` : t(i18nKey);
    listEl.appendChild(hdr);
    for (const s of group) {
      listEl.appendChild(buildSessionRow(s, activeKey));
    }
  }
}

async function loadSessions() {
  const res = await apiFetch("/dashboard/api/sessions");
  if (!res.ok) return;
  const data = await res.json();
  sessions = data.sessions || [];
  if (sessions.length > 0) {
    chat.markHasExistingSessions();
  }
  const wp = data.warm_pools || {};
  const wpEl = document.getElementById("warm-pool-pill");
  if (wpEl) wpEl.textContent = `${Number(wp.total_active || 0)}`;

  const activeKey = chat.getActiveSessionKey();
  if (activeKey) {
    const fresh = sessions.find((s) => s.session_key === activeKey);
    if (fresh) {
      chat.handleWsEvent({
        type: "session_updated",
        session_key: fresh.session_key,
        provider: fresh.provider,
        model: fresh.model,
        effort: fresh.effort,
        topic_name: fresh.topic_name,
        pinned: fresh.pinned,
        busy: fresh.busy,
        asking: fresh.asking
      });
    } else {
      await chat.selectSession(null);
    }
  }
  renderSessionList();
}

async function refreshHitlBanner() {
  const res = await apiFetch("/dashboard/api/hitl");
  if (!res.ok) return;
  const data = await res.json();
  activeHitlSessions = data.sessions || [];
  renderHitlBannerDom();
}

function renderHitlBannerDom() {
  const bannerEl = document.getElementById("hitl-banner");
  const labelEl = document.getElementById("hitl-banner-label");
  const dotEl = document.getElementById("tab-browser-dot");
  const active = activeHitlSessions.length > 0;
  dotEl?.classList.toggle("hidden", !active);
  if (!bannerEl || !labelEl) return;
  const browserVisibleNow = !inspector.isCollapsed() && inspector.getActiveTab() === "browser";
  if (!active || browserVisibleNow) {
    bannerEl.classList.add("hidden");
    return;
  }
  const names = activeHitlSessions.map((s) => s.site_key || "auth").join(", ");
  labelEl.textContent = `🌐 ${t("hitlBannerText", { site: names })}`;
  bannerEl.classList.remove("hidden");
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
          if (typeof ev.pinned === "boolean") target.pinned = ev.pinned;
          target.busy = ev.busy;
          target.asking = ev.asking;
          renderSessionList();
        } else {
          loadSessions();
        }
      } else if (ev.type === "turn_result") {
        loadSessions();
        if (!inspector.isCollapsed() && inspector.getActiveTab() === "tree") {
          inspector.refreshCurrentTab();
        }
      } else if (ev.type === "cron_updated" && !inspector.isCollapsed() && inspector.getActiveTab() === "cron") {
        inspector.refreshCurrentTab();
      } else if (ev.type === "hitl_updated") {
        refreshHitlBanner();
        inspector.onHitlUpdated();
      } else if (ev.type === "provider_setup_updated") {
        inspector.onProviderSetupUpdated();
        chat.onProviderSetupUpdated(ev.provider);
      } else if (ev.type === "telegram_status_updated") {
        telegramConnected = Boolean(ev.connected);
        telegramBotUsername = ev.bot_username || "";
        chat.setTelegramState(telegramConnected, telegramBotUsername);
        renderSessionList();
        inspector.onProviderSetupUpdated();
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

async function openDraftMode() {
  confirmingDeleteKey = "";
  renamingKey = "";
  closeDrawers();
  await chat.selectSession(null);
  renderSessionList();
  chat.focusComposer();
}

function setupUiListeners() {
  document.getElementById("session-search").addEventListener("input", renderSessionList);

  document.querySelectorAll("#sidebar-transport-tabs .seg-tab").forEach((tabBtn) => {
    tabBtn.addEventListener("click", async () => {
      const nextTr = tabBtn.dataset.transport || "web";
      if (nextTr === activeTransport) return;
      activeTransport = nextTr;
      renamingKey = "";
      confirmingDeleteKey = "";
      renderSessionList();
      const activeKey = chat.getActiveSessionKey();
      const activeObj = activeKey ? sessions.find((s) => s.session_key === activeKey) : null;
      if (!activeObj || activeObj.transport !== activeTransport) {
        await chat.selectSession(null);
        renderSessionList();
      } else {
        chat.refreshDraftTransportUi();
      }
    });
  });

  document.getElementById("btn-new-topic").addEventListener("click", openDraftMode);

  const pruneBtn = document.getElementById("btn-prune-sessions");
  pruneBtn?.addEventListener("click", async () => {
    const res = await apiFetch("/dashboard/api/sessions/prune", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ transport: activeTransport, older_than_days: 30 })
    });
    if (res.ok) {
      const d = await res.json();
      const count = Number(d.pruned_count ?? d.removed_count ?? 0);
      await loadSessions();
      pruneBtn.textContent = t("pruneDoneToast", { count });
      setTimeout(() => { pruneBtn.textContent = t("pruneBtn"); }, 1800);
    }
  });

  document.getElementById("btn-hitl-open")?.addEventListener("click", () => {
    const tok = activeHitlSessions[0]?.token || "";
    inspector.openBrowserSession(tok);
  });

  window.addEventListener("message", (ev) => {
    if (ev.origin !== window.location.origin) return;
    if (ev.data && ev.data.type === "tuner_hitl_done") {
      inspector.notifyBrowserDone(ev.data.site_key || "site");
      refreshHitlBanner();
    } else if (ev.data && ev.data.type === "tuner_hitl_clipboard" && ev.data.text) {
      try { navigator.clipboard?.writeText(ev.data.text).catch(() => {}); } catch (_) {}
      const codeInp = document.querySelector(".oauth-code-input");
      if (codeInp && !codeInp.value) {
        codeInp.value = ev.data.text;
      }
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
    renderHitlBannerDom();
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
  telegramConnected = Boolean(me.telegram_connected);
  telegramBotUsername = me.bot_username || "";
  initLang(me.language);
  applyDomI18n();

  chat.initDraftDefaults(me.default_provider, me.default_model, me.needs_onboarding, me.ready_providers);
  chat.setTelegramState(telegramConnected, telegramBotUsername);

  const profEl = document.getElementById("profile-badge");
  if (profEl) {
    profEl.textContent = me.profile || "default";
    profEl.classList.toggle("hidden", !me.profile || me.profile === "default");
  }

  setupUiListeners();
  await Promise.all([loadSessions(), chat.selectSession(null), refreshHitlBanner()]);
  renderSessionList();
  connectWebSocket();

  if ("serviceWorker" in navigator) {
    navigator.serviceWorker.register("/dashboard/sw.js", { scope: "/dashboard" }).catch(() => {});
  }
}

boot();
