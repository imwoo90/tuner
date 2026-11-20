import { escapeHtml, renderMarkdown } from "/dashboard/assets/markdown.js";
import { t, setLang, toggleLang, onLangChange } from "/dashboard/assets/i18n.js";
import { createOnboardingRenderer } from "/dashboard/assets/onboarding.js";
import {
  createSlashController,
  describeCommand,
  parseBuiltinSlashInput
} from "/dashboard/assets/slash.js";

export function createChatController({
  apiFetch,
  onOpenFile,
  onOpenInspectorTab,
  onOpenBrowserToken,
  onRefreshSessions,
  onTelegramConnected,
  getActiveTransport,
  getPreferredTgChatId
}) {
  const els = {
    scrollWrap: document.getElementById("chat-scroll"),
    composerShell: document.querySelector(".composer-shell"),
    messages: document.getElementById("chat-messages"),
    liveBubble: document.getElementById("live-stream-bubble"),
    liveBody: document.getElementById("live-stream-body"),
    askCard: document.getElementById("ask-card"),
    askProgress: document.getElementById("ask-progress"),
    askQuestion: document.getElementById("ask-question-text"),
    askOptions: document.getElementById("ask-options"),
    askWriteinInput: document.getElementById("ask-writein-input"),
    askWriteinBtn: document.getElementById("ask-writein-btn"),
    askPrevBtn: document.getElementById("ask-prev-btn"),
    askSubmitBtn: document.getElementById("ask-submit-btn"),
    askSkipBtn: document.getElementById("ask-skip-btn"),
    topicTitle: document.getElementById("active-topic-title"),
    channelBadge: document.getElementById("active-channel-badge"),
    topicKey: document.getElementById("active-topic-key"),
    topicState: document.getElementById("active-topic-state"),
    selProvider: document.getElementById("sel-provider"),
    selModel: document.getElementById("sel-model"),
    selEffort: document.getElementById("sel-effort"),
    input: document.getElementById("chat-input"),
    slashMenu: document.getElementById("slash-menu"),
    slashBtn: document.getElementById("btn-slash"),
    sendBtn: document.getElementById("btn-send"),
    attachBtn: document.getElementById("btn-attach"),
    fileInput: document.getElementById("file-upload-input"),
    pendingWrap: document.getElementById("pending-attachments"),
    stopBtn: document.getElementById("btn-stop"),
    newBtn: document.getElementById("btn-new"),
    chatPane: document.getElementById("chat-pane"),
    dropOverlay: document.getElementById("drop-overlay")
  };

  const ALL_PROVIDERS = ["antigravity", "claude", "codex"];
  const mobileMq = typeof window !== "undefined" && window.matchMedia ? window.matchMedia("(max-width: 640px)") : null;

  function isCompactMobile() {
    return Boolean(mobileMq?.matches);
  }

  function formatProviderChipLabel(prov) {
    return isCompactMobile() && prov === "antigravity" ? "agy" : prov;
  }

  function syncComposerPlaceholder() {
    const ask = state.activeAsk;
    const hasAsk = !state.managingEngines && ask && Array.isArray(ask.questions) && ask.questions.length > 0;
    if (hasAsk) {
      els.input.placeholder = t("composerAskPlaceholder");
    } else {
      els.input.placeholder = t(isCompactMobile() ? "composerPlaceholderMobile" : "composerPlaceholder");
    }
  }

  let state = {
    session: null,
    needsOnboarding: false,
    managingEngines: false,
    readyProviders: [],
    telegramConnected: true,
    telegramBotUsername: "",
    draft: { provider: "antigravity", model: "", effort: "" },
    messages: [],
    liveStream: "",
    activeAsk: null,
    selectedIndices: new Set(),
    pendingFiles: [],
    modelCatalog: {},
    commandCatalog: {}
  };

  function activeProv() {
    return (state.session ? state.session.provider : state.draft.provider) || "antigravity";
  }

  function deriveAutoTopicTitle(rawText = "", files = []) {
    let cleaned = String(rawText || "")
      .replace(/^@\S+\s+/, "")
      .replace(/^\/[a-zA-Z0-9_-]+\s+/, "")
      .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
      .replace(/(?:file|https?):\/\/\S+/g, (u) => u.split("/").filter(Boolean).pop() || "");
    const firstLine = cleaned
      .split(/\r?\n/)
      .map((l) => l.replace(/\s+/g, " ").trim())
      .find((l) => l.length > 0);
    if (firstLine) {
      return firstLine.length > 36 ? `${firstLine.slice(0, 35).trimEnd()}…` : firstLine;
    }
    if (files && files.length > 0) {
      const fname = String(files[0]).split("/").pop() || "file";
      return `📎 ${fname.slice(0, 30)}`;
    }
    return null;
  }

  function isNearBottom() {
    const sc = els.scrollWrap || els.messages;
    return sc.scrollHeight - sc.scrollTop - sc.clientHeight < 140;
  }

  function scrollToBottom() {
    const sc = els.scrollWrap || els.messages;
    sc.scrollTop = sc.scrollHeight;
  }

  function renderSingleMessage(m) {
    const div = document.createElement("div");
    const isUser = m.sender === "user";
    const errCls = !isUser && m.is_success === false ? " msg-err" : "";
    div.className = `msg-bubble ${isUser ? "msg-user" : "msg-bot"}${errCls}`;

    const ts = m.timestamp ? new Date(m.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "";
    let filesHtml = "";
    if (Array.isArray(m.files) && m.files.length > 0) {
      filesHtml = `<div style="margin-top:8px;">${m.files
        .map((f) => {
          const base = f.split("/").pop() || f;
          return `<button type="button" class="file-chip" data-file-path="${escapeHtml(f)}">📄 ${escapeHtml(base)}</button>`;
        })
        .join("")}</div>`;
    }

    if (isUser) {
      div.title = ts;
      div.innerHTML = `<div class="msg-body">${renderMarkdown(m.text || "")}</div>${filesHtml}`;
    } else {
      div.innerHTML = `
        <div class="msg-meta">
          <span>Tuner</span>
          <span class="msg-time">${escapeHtml(ts)}</span>
        </div>
        <div class="msg-body">${renderMarkdown(m.text || "")}</div>
        ${filesHtml}`;
    }
    return div;
  }

  async function refreshReadyProviders() {
    try {
      const r = await apiFetch("/dashboard/api/providers/status");
      if (!r.ok) return;
      const d = await r.json();
      state.readyProviders = (d.providers || [])
        .filter((p) => p.installed && p.authenticated)
        .map((p) => p.id);
      populateModelAndEffortControls();
    } catch (_) {}
  }

  function openEngineManager() {
    state.managingEngines = true;
    slashCtrl.closeMenu();
    renderHeader();
    renderMessages(false);
    renderLiveStream();
    renderAskCard();
  }

  function closeEngineManager() {
    state.managingEngines = false;
    renderHeader();
    renderMessages(true);
    renderLiveStream();
    renderAskCard();
    els.input?.focus();
  }

  const wizardRenderer = createOnboardingRenderer({
    apiFetch,
    onOpenBrowserToken: (tok) => onOpenBrowserToken?.(tok),
    onCloseManager: closeEngineManager,
    onProviderReady: async (readyProv) => {
      state.needsOnboarding = false;
      state.managingEngines = false;
      await refreshReadyProviders();
      const targetProv = readyProv || activeProv();
      delete state.modelCatalog[targetProv];
      if (state.session) {
        if (state.session.provider !== targetProv) {
          await updateConfig({ provider: targetProv });
        } else {
          await ensureModelsLoaded(targetProv);
        }
        renderHeader();
        renderMessages(true);
      } else {
        state.draft.provider = targetProv;
        state.draft.model = "";
        state.draft.effort = "";
        await ensureModelsLoaded(targetProv);
        renderHeader();
        renderMessages(false);
      }
      renderLiveStream();
      renderAskCard();
      els.input?.focus();
    }
  });

  function syncComposerVisibility() {
    const isTg = (typeof getActiveTransport === "function" ? getActiveTransport() : "web") === "tg";
    const hide = state.managingEngines || (!state.session && (state.needsOnboarding || (isTg && !state.telegramConnected)));
    els.composerShell?.classList.toggle("hidden", hide);
  }

  function renderTelegramConnectCard() {
    els.messages.innerHTML = `<div id="tg-connect-card" class="onboarding-wrap">
      <div class="onboard-header">
        <h2>✈️ ${escapeHtml(t("tgSetupHeroTitle"))}</h2>
        <p>${escapeHtml(t("tgSetupHeroSub"))}</p>
      </div>
      <form id="tg-connect-form" class="card tg-setup-form">
        <div>
          <label class="tg-field-label" for="tg-token-input">${escapeHtml(t("tgTokenLabel"))}</label>
          <input id="tg-token-input" type="password" placeholder="${escapeHtml(t("tgTokenPh"))}" autocomplete="off" required>
        </div>
        <div>
          <label class="tg-field-label" for="tg-allowed-input">${escapeHtml(t("tgAllowedLabel"))}</label>
          <input id="tg-allowed-input" type="text" placeholder="${escapeHtml(t("tgAllowedPh"))}" autocomplete="off">
          <div class="muted" style="margin-top:4px;font-size:11.5px;">${escapeHtml(t("tgAllowedHelp"))}</div>
        </div>
        <div id="tg-connect-err" class="alert alert-danger hidden"></div>
        <div class="card-row" style="justify-content:flex-end;">
          <button id="btn-tg-connect" class="btn btn-primary" type="submit">${escapeHtml(t("tgConnectBtn"))}</button>
        </div>
      </form>
    </div>`;

    const form = els.messages.querySelector("#tg-connect-form");
    form?.addEventListener("submit", async (e) => {
      e.preventDefault();
      const btn = els.messages.querySelector("#btn-tg-connect");
      const errEl = els.messages.querySelector("#tg-connect-err");
      const token = (els.messages.querySelector("#tg-token-input")?.value || "").trim();
      const allowed = (els.messages.querySelector("#tg-allowed-input")?.value || "").trim();
      if (!token) return;
      errEl?.classList.add("hidden");
      if (btn) {
        btn.disabled = true;
        btn.textContent = t("tgConnectingBtn");
      }
      const r = await apiFetch("/dashboard/api/telegram/connect", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ token, allowed_ids: allowed || null })
      });
      const d = await r.json().catch(() => ({}));
      if (!r.ok) {
        if (btn) {
          btn.disabled = false;
          btn.textContent = t("tgConnectBtn");
        }
        if (errEl) {
          errEl.textContent = d.error || `HTTP ${r.status}`;
          errEl.classList.remove("hidden");
        }
        return;
      }
      state.telegramConnected = true;
      state.telegramBotUsername = d.bot_username || "";
      onTelegramConnected?.(state.telegramBotUsername);
      renderHeader();
      renderMessages(false);
    });
  }

  function renderMessages(forceScroll = false) {
    syncComposerVisibility();
    const stick = forceScroll || isNearBottom();
    els.messages.innerHTML = "";
    if (state.managingEngines) {
      wizardRenderer.renderInto(els.messages, { isWizard: false, isManager: true });
      return;
    }
    if (!state.session) {
      if (state.needsOnboarding) {
        wizardRenderer.renderInto(els.messages, { isWizard: true });
        return;
      }
      wizardRenderer.clearPoll();
      const isTg = (typeof getActiveTransport === "function" ? getActiveTransport() : "web") === "tg";
      if (isTg && !state.telegramConnected) {
        renderTelegramConnectCard();
        return;
      }
      const hTitle = isTg ? t("draftHeroTitleTg") : t("draftHeroTitleWeb");
      const hSub = isTg ? t("draftHeroSubTg") : t("draftHeroSubWeb");
      els.messages.innerHTML = `<div id="draft-hero" class="draft-hero"><h2>${escapeHtml(hTitle)}</h2><p>${escapeHtml(hSub)}</p></div>`;
      return;
    }
    wizardRenderer.clearPoll();
    if (state.messages.length === 0) {
      els.messages.innerHTML = `<div class="empty-state">${escapeHtml(t("emptyNoMessages"))}</div>`;
      return;
    }
    for (const m of state.messages) {
      els.messages.appendChild(renderSingleMessage(m));
    }
    if (stick) scrollToBottom();
  }

  function renderLiveStream() {
    if (state.managingEngines || !state.liveStream) {
      els.liveBubble.classList.add("hidden");
      if (!state.liveStream) els.liveBody.innerHTML = "";
      return;
    }
    const stick = isNearBottom();
    els.liveBubble.classList.remove("hidden");
    els.liveBody.innerHTML = renderMarkdown(state.liveStream);
    if (stick) scrollToBottom();
  }

  function renderAskCard() {
    const ask = state.activeAsk;
    syncComposerPlaceholder();
    if (state.managingEngines || !ask || !Array.isArray(ask.questions) || ask.questions.length === 0) {
      els.askCard.classList.add("hidden");
      return;
    }
    const idx = Math.min(ask.current_index || 0, ask.questions.length - 1);
    const q = ask.questions[idx];
    state.selectedIndices.clear();
    els.askCard.classList.remove("hidden");
    els.askProgress.textContent = `${idx + 1} / ${ask.questions.length}`;
    els.askQuestion.innerHTML = renderMarkdown(q.question || "");
    els.askOptions.innerHTML = "";
    els.askWriteinInput.value = "";
    els.askPrevBtn.classList.toggle("hidden", idx === 0);
    els.askSubmitBtn.classList.toggle("hidden", !q.is_multi_select);

    (q.options || []).forEach((opt, optIdx) => {
      const lower = opt.toLowerCase();
      if (lower.includes("write-in") || opt.includes("직접 입력")) return;
      if (q.is_multi_select) {
        const lbl = document.createElement("label");
        lbl.className = "ask-check";
        const cb = document.createElement("input");
        cb.type = "checkbox";
        cb.addEventListener("change", () => {
          if (cb.checked) state.selectedIndices.add(optIdx);
          else state.selectedIndices.delete(optIdx);
        });
        lbl.appendChild(cb);
        lbl.appendChild(document.createTextNode(opt));
        els.askOptions.appendChild(lbl);
      } else {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "btn btn-sm ask-opt-btn";
        btn.textContent = opt;
        btn.addEventListener("click", () => submitAsk({ action: "answer", selected_indices: [optIdx] }));
        els.askOptions.appendChild(btn);
      }
    });
    scrollToBottom();
  }

  function renderPendingFiles() {
    if (!state.pendingFiles.length) {
      els.pendingWrap.classList.add("hidden");
      els.pendingWrap.innerHTML = "";
      return;
    }
    els.pendingWrap.classList.remove("hidden");
    els.pendingWrap.innerHTML = state.pendingFiles
      .map((f) => `<span class="pill">📎 ${escapeHtml(f.split("/").pop() || f)}</span>`)
      .join("");
  }

  function populateModelAndEffortControls() {
    if (!state.session && state.readyProviders.length > 0 && !state.readyProviders.includes(state.draft.provider)) {
      state.draft.provider = state.readyProviders[0];
      state.draft.model = "";
      state.draft.effort = "";
    }
    const target = state.session || state.draft;
    const prov = target.provider || "antigravity";

    const readySet = new Set(state.readyProviders);
    els.selProvider.innerHTML = "";
    for (const p of ALL_PROVIDERS) {
      if (readySet.has(p)) {
        const opt = document.createElement("option");
        opt.value = p;
        opt.textContent = formatProviderChipLabel(p);
        els.selProvider.appendChild(opt);
      }
    }
    if (!readySet.has(prov)) {
      const fallbackOpt = document.createElement("option");
      fallbackOpt.value = prov;
      fallbackOpt.disabled = true;
      fallbackOpt.textContent = `${formatProviderChipLabel(prov)} (${t("provNeedAuthBadge")})`;
      els.selProvider.appendChild(fallbackOpt);
    }
    if (els.selProvider.options.length > 0) {
      els.selProvider.appendChild(document.createElement("hr"));
    }
    const unready = ALL_PROVIDERS.filter((p) => !readySet.has(p));
    const mgrOpt = document.createElement("option");
    mgrOpt.value = "__manage_engines__";
    mgrOpt.textContent = unready.length > 0
      ? t("provDropdownAdd", { list: unready.join(" · ") })
      : t("provDropdownManageAll");
    els.selProvider.appendChild(mgrOpt);
    els.selProvider.value = prov;

    const cat = state.modelCatalog[prov] || [];
    els.selModel.innerHTML = "";
    let hasCurrent = false;
    for (const item of cat) {
      const opt = document.createElement("option");
      opt.value = item.model;
      opt.textContent = item.model;
      if (item.model === target.model) hasCurrent = true;
      els.selModel.appendChild(opt);
    }
    if (!state.session && !hasCurrent && cat.length > 0) {
      state.draft.model = cat[0].model;
      hasCurrent = true;
    }
    if (!hasCurrent && target.model) {
      const opt = document.createElement("option");
      opt.value = target.model;
      opt.textContent = target.model;
      els.selModel.appendChild(opt);
    }
    els.selModel.value = target.model || (cat[0]?.model || "");

    const modelEntry = cat.find((m) => m.model === els.selModel.value);
    const efforts = modelEntry ? modelEntry.efforts || [] : [];
    els.selEffort.innerHTML = "";
    if (!efforts.length) {
      els.selEffort.classList.add("hidden");
      els.selEffort.disabled = true;
      if (!state.session) state.draft.effort = "";
    } else {
      els.selEffort.classList.remove("hidden");
      els.selEffort.disabled = false;
      for (const ef of efforts) {
        const opt = document.createElement("option");
        opt.value = ef;
        opt.textContent = ef;
        els.selEffort.appendChild(opt);
      }
      const chosenEff = target.effort && efforts.includes(target.effort) ? target.effort : efforts[0];
      els.selEffort.value = chosenEff;
      if (!state.session) state.draft.effort = chosenEff;
    }
  }

  function formatTopicLabel(s) {
    if (s.topic_name) return s.topic_name;
    return s.transport === "web"
      ? t("webTopicFallback", { id: s.topic_id })
      : t("tgTopicFallback", { id: s.topic_id || s.chat_id });
  }

  function renderHeader() {
    syncComposerVisibility();
    if (state.managingEngines) {
      els.topicTitle.textContent = t("engineMgrTitle");
      if (els.channelBadge) els.channelBadge.className = "pill hidden";
      els.topicKey.textContent = "";
      els.topicState.dataset.state = "Idle";
      els.topicState.textContent = "Idle";
      els.topicState.className = "status-badge hidden";
      els.stopBtn.classList.add("hidden");
      els.newBtn?.classList.add("hidden");
      populateModelAndEffortControls();
      return;
    }
    const s = state.session;
    if (!s) {
      const isTg = (typeof getActiveTransport === "function" ? getActiveTransport() : "web") === "tg";
      if (state.needsOnboarding) {
        els.topicTitle.textContent = t("onboardHeroTitle");
        if (els.channelBadge) els.channelBadge.className = "pill hidden";
      } else if (isTg && !state.telegramConnected) {
        els.topicTitle.textContent = t("tgSetupHeaderTitle");
        if (els.channelBadge) {
          els.channelBadge.textContent = t("tgNotConnectedBadge");
          els.channelBadge.className = "pill";
        }
      } else {
        els.topicTitle.textContent = isTg ? t("draftTitleTg") : t("draftTitleWeb");
        if (els.channelBadge) {
          els.channelBadge.textContent = isTg
            ? (state.telegramBotUsername ? `✈️ @${state.telegramBotUsername}` : t("channelBadgeTg"))
            : t("channelBadgeWeb");
          els.channelBadge.className = isTg ? "pill pill-info" : "pill";
        }
      }
      els.topicKey.textContent = "";
      els.topicState.dataset.state = "Idle";
      els.topicState.textContent = "Idle";
      els.topicState.className = "status-badge hidden";
      els.stopBtn.classList.add("hidden");
      els.newBtn?.classList.add("hidden");
      populateModelAndEffortControls();
      return;
    }
    els.newBtn?.classList.remove("hidden");
    els.topicTitle.textContent = formatTopicLabel(s);
    if (els.channelBadge) {
      els.channelBadge.textContent = s.transport === "tg"
        ? (state.telegramBotUsername ? `✈️ @${state.telegramBotUsername}` : t("channelBadgeTg"))
        : t("channelBadgeWeb");
      els.channelBadge.className = s.transport === "tg" ? "pill pill-info" : "pill";
    }
    els.topicKey.textContent = s.session_key;
    if (s.asking) {
      els.topicState.dataset.state = "Asking";
      els.topicState.textContent = "Asking";
      els.topicState.className = "status-badge pill-warn";
      els.stopBtn.classList.remove("hidden");
    } else if (s.busy) {
      els.topicState.dataset.state = "Running";
      els.topicState.textContent = "Running";
      els.topicState.className = "status-badge pill-ok";
      els.stopBtn.classList.remove("hidden");
    } else {
      els.topicState.dataset.state = "Idle";
      els.topicState.textContent = "Idle";
      els.topicState.className = "status-badge hidden";
      els.stopBtn.classList.add("hidden");
    }
    populateModelAndEffortControls();
  }

  async function ensureModelsLoaded(provider) {
    if (state.modelCatalog[provider] && state.commandCatalog[provider]) return;
    const res = await apiFetch(`/dashboard/api/models?provider=${encodeURIComponent(provider)}`);
    if (res.ok) {
      const data = await res.json();
      state.modelCatalog[provider] = data.models || [];
      state.commandCatalog[provider] = data.commands || [];
    }
  }

  async function selectSession(session) {
    state.managingEngines = false;
    state.session = session;
    if (session) {
      state.draft.provider = session.provider || state.draft.provider;
      state.draft.model = session.model || "";
      state.draft.effort = session.effort || "";
    }
    state.messages = [];
    state.liveStream = "";
    state.activeAsk = null;
    state.pendingFiles = session?.pending_attachments ? [...session.pending_attachments] : [];
    slashCtrl.closeMenu();
    renderHeader();
    renderMessages(true);
    renderLiveStream();
    renderAskCard();
    renderPendingFiles();
    if (!session) {
      await ensureModelsLoaded(state.draft.provider);
      populateModelAndEffortControls();
      return;
    }

    await ensureModelsLoaded(session.provider);
    populateModelAndEffortControls();

    const res = await apiFetch(`/dashboard/api/sessions/history?session_key=${encodeURIComponent(session.session_key)}`);
    if (res.ok) {
      const data = await res.json();
      state.messages = data.messages || [];
      state.liveStream = data.live_stream || "";
      state.activeAsk = data.active_ask || null;
      if (data.session) {
        state.session = data.session;
        state.pendingFiles = data.session.pending_attachments || [];
      }
      renderHeader();
      renderMessages(true);
      renderLiveStream();
      renderAskCard();
      renderPendingFiles();
    }
  }

  async function stopCurrentSession() {
    if (!state.session) return;
    await apiFetch("/dashboard/api/sessions/stop", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_key: state.session.session_key })
    });
  }

  async function resetCurrentSession() {
    if (!state.session) return;
    const res = await apiFetch("/dashboard/api/sessions/new", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_key: state.session.session_key })
    });
    if (res.ok) {
      await selectSession(state.session);
      onRefreshSessions();
    }
  }

  function appendLocalSystemNotice(markdown) {
    state.messages.push({
      sender: "bot",
      text: markdown,
      timestamp: new Date().toISOString(),
      is_success: true
    });
    renderMessages(true);
  }

  async function runBuiltinSlash(cmd, args) {
    if (cmd === "usage") return onOpenInspectorTab?.("usage");
    if (cmd === "cron") return onOpenInspectorTab?.("cron");
    if (cmd === "vault") return onOpenInspectorTab?.("vault");
    if (cmd === "auth") return onOpenInspectorTab?.("browser");
    if (cmd === "security") return onOpenInspectorTab?.("security");
    if (cmd === "status") return onOpenInspectorTab?.("tree");
    if (cmd === "lang") {
      if (args) setLang(args);
      else toggleLang();
      if (state.session) appendLocalSystemNotice(t("langChangedNotice"));
      return;
    }
    if (cmd === "provider") {
      const p = args.toLowerCase();
      const norm = (p === "agy" || p === "antigravity") ? "antigravity" : (p === "claude" || p === "codex") ? p : "";
      if (norm) {
        if (state.readyProviders.length > 0 && !state.readyProviders.includes(norm)) {
          openEngineManager();
          return;
        }
        return updateConfig({ provider: norm });
      }
      openEngineManager();
      return;
    }
    if (cmd === "model") {
      if (args) return updateConfig({ model: args });
      els.selModel.focus();
      return;
    }
    if (cmd === "effort") {
      if (args) return updateConfig({ effort: args.toLowerCase() });
      if (!els.selEffort.classList.contains("hidden")) els.selEffort.focus();
      return;
    }
    if (cmd === "new" || cmd === "reset") {
      if (!state.session) return;
      return resetCurrentSession();
    }
    if (cmd === "stop" || cmd === "abort") {
      if (!state.session) return;
      return stopCurrentSession();
    }
    if (cmd === "help") {
      const prov = activeProv();
      const cmds = state.commandCatalog[prov] || [];
      const lines = cmds.map((c) => `- \`/${c.command}\` — ${describeCommand(c)}`).join("\n");
      if (state.session) {
        appendLocalSystemNotice(`### ${t("helpHeader", { prov })}\n${lines}\n\n${t("helpTip")}`);
      } else {
        els.messages.innerHTML = `<div class="msg-bubble msg-bot"><div class="msg-body">${renderMarkdown(`### ${t("helpHeader", { prov })}\n${lines}\n\n${t("helpTip")}`)}</div></div>`;
      }
    }
  }

  const slashCtrl = createSlashController({
    inputEl: els.input,
    menuEl: els.slashMenu,
    slashBtnEl: els.slashBtn,
    getProvider: () => activeProv(),
    getCommands: () => state.commandCatalog[activeProv()] || [],
    getModels: () => state.modelCatalog[activeProv()] || [],
    onRunBuiltin: runBuiltinSlash
  });

  async function ensureActiveSession(seedText = "") {
    if (state.session) return state.session;
    const title = deriveAutoTopicTitle(seedText, state.pendingFiles);
    const prov = state.draft.provider || els.selProvider?.value || "antigravity";
    const model = state.draft.model || els.selModel?.value || null;
    const effort = !els.selEffort?.disabled && els.selEffort?.value ? els.selEffort.value : null;
    const tr = typeof getActiveTransport === "function" ? getActiveTransport() : "web";
    const prefChat = tr === "tg" && typeof getPreferredTgChatId === "function" ? getPreferredTgChatId() : null;
    const res = await apiFetch("/dashboard/api/sessions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        transport: tr,
        chat_id: prefChat,
        silent_greeting: true,
        topic_name: title,
        provider: prov,
        model,
        effort
      })
    });
    if (!res.ok) return null;
    const created = await res.json();
    state.needsOnboarding = false;
    state.session = created;
    renderHeader();
    renderMessages(false);
    await onRefreshSessions?.();
    return created;
  }

  async function sendPrompt() {
    const text = els.input.value.trim();
    if (!text && !state.pendingFiles.length) return;

    if (!state.activeAsk && !state.pendingFiles.length) {
      const builtin = parseBuiltinSlashInput(text);
      if (builtin) {
        els.input.value = "";
        els.input.style.height = "auto";
        slashCtrl.closeMenu();
        await runBuiltinSlash(builtin.command, builtin.args);
        return;
      }
    }

    if (!state.session) {
      const created = await ensureActiveSession(text);
      if (!created) return;
    }

    els.input.value = "";
    els.input.style.height = "auto";
    slashCtrl.closeMenu();
    const res = await apiFetch("/dashboard/api/sessions/send", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_key: state.session.session_key, text })
    });
    if (res.ok) {
      state.pendingFiles = [];
      renderPendingFiles();
    }
  }

  async function submitAsk(payload) {
    if (!state.session) return;
    await apiFetch("/dashboard/api/sessions/ask", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_key: state.session.session_key, ...payload })
    });
  }

  async function uploadFiles(fileList) {
    if (!fileList || !fileList.length) return;
    if (!state.session) {
      const created = await ensureActiveSession(fileList[0]?.name || "");
      if (!created) return;
    }
    for (const file of fileList) {
      const fd = new FormData();
      fd.append("session_key", state.session.session_key);
      fd.append("file", file);
      const res = await apiFetch("/dashboard/api/sessions/upload", { method: "POST", body: fd });
      if (res.ok) {
        const data = await res.json();
        state.pendingFiles = data.pending_attachments || [...state.pendingFiles, data.path];
        renderPendingFiles();
      }
    }
  }

  async function updateConfig(patch) {
    if (!state.session) {
      if (patch.provider) {
        state.draft.provider = patch.provider;
        state.draft.model = "";
        state.draft.effort = "";
        await ensureModelsLoaded(patch.provider);
      }
      if (patch.model !== undefined) state.draft.model = patch.model;
      if (patch.effort !== undefined) state.draft.effort = patch.effort;
      populateModelAndEffortControls();
      slashCtrl.refresh();
      return;
    }
    const res = await apiFetch("/dashboard/api/sessions/config", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_key: state.session.session_key, ...patch })
    });
    if (res.ok) {
      const updated = await res.json();
      state.session = updated;
      state.draft.provider = updated.provider || state.draft.provider;
      state.draft.model = updated.model || "";
      state.draft.effort = updated.effort || "";
      await ensureModelsLoaded(updated.provider);
      renderHeader();
      onRefreshSessions();
    }
  }

  function handleWsEvent(ev) {
    if (ev.type === "session_updated" && state.session && ev.session_key === state.session.session_key) {
      state.session.provider = ev.provider;
      state.session.model = ev.model;
      state.session.effort = ev.effort;
      state.session.topic_name = ev.topic_name;
      state.session.busy = ev.busy;
      state.session.asking = ev.asking;
      if (!ev.asking && state.activeAsk) {
        state.activeAsk = null;
        renderAskCard();
      }
      if (!ev.busy && state.liveStream) {
        state.liveStream = "";
        renderLiveStream();
      }
      ensureModelsLoaded(ev.provider).then(renderHeader);
    }
    if (!state.session || ev.session_key !== state.session.session_key) return;
    if (ev.type === "user_message") {
      state.messages.push({ sender: "user", text: ev.text, timestamp: ev.timestamp, is_success: true });
      renderMessages(true);
    } else if (ev.type === "stream_delta") {
      state.liveStream = ev.text || "";
      renderLiveStream();
    } else if (ev.type === "ask_question") {
      state.activeAsk = ev;
      state.session.asking = true;
      renderHeader();
      renderAskCard();
    } else if (ev.type === "ask_resolved") {
      state.activeAsk = null;
      state.session.asking = false;
      renderHeader();
      renderAskCard();
    } else if (ev.type === "turn_result") {
      state.liveStream = "";
      renderLiveStream();
      if (ev.text) {
        state.messages.push({
          sender: "bot",
          text: ev.text,
          files: ev.files || [],
          timestamp: ev.timestamp,
          is_success: !ev.is_error
        });
        renderMessages(true);
      }
    }
  }

  onLangChange(() => {
    renderHeader();
    renderMessages(false);
    renderLiveStream();
    renderAskCard();
    slashCtrl.refresh();
  });

  mobileMq?.addEventListener?.("change", () => {
    populateModelAndEffortControls();
    syncComposerPlaceholder();
  });

  els.sendBtn.addEventListener("click", sendPrompt);
  els.input.addEventListener("keydown", (e) => {
    if (slashCtrl.handleKeyDown(e)) return;
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      sendPrompt();
    }
  });
  els.input.addEventListener("input", () => {
    els.input.style.height = "auto";
    els.input.style.height = `${Math.min(els.input.scrollHeight, 160)}px`;
  });

  els.attachBtn.addEventListener("click", () => els.fileInput.click());
  els.fileInput.addEventListener("change", () => {
    uploadFiles(els.fileInput.files);
    els.fileInput.value = "";
  });

  els.chatPane.addEventListener("dragover", (e) => {
    e.preventDefault();
    els.dropOverlay.classList.remove("hidden");
  });
  els.chatPane.addEventListener("dragleave", (e) => {
    if (e.target === els.dropOverlay) els.dropOverlay.classList.add("hidden");
  });
  els.chatPane.addEventListener("drop", (e) => {
    e.preventDefault();
    els.dropOverlay.classList.add("hidden");
    if (e.dataTransfer?.files?.length) uploadFiles(e.dataTransfer.files);
  });

  els.selProvider.addEventListener("change", () => {
    if (els.selProvider.value === "__manage_engines__") {
      els.selProvider.value = activeProv();
      openEngineManager();
      return;
    }
    updateConfig({ provider: els.selProvider.value });
  });
  els.selModel.addEventListener("change", () => updateConfig({ model: els.selModel.value }));
  els.selEffort.addEventListener("change", () => updateConfig({ effort: els.selEffort.value }));

  els.stopBtn.addEventListener("click", stopCurrentSession);
  els.newBtn.addEventListener("click", resetCurrentSession);

  els.askWriteinBtn.addEventListener("click", () => {
    const val = els.askWriteinInput.value.trim();
    if (val) submitAsk({ action: "answer", answer: val });
  });
  els.askWriteinInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      const val = els.askWriteinInput.value.trim();
      if (val) submitAsk({ action: "answer", answer: val });
    }
  });
  els.askPrevBtn.addEventListener("click", () => submitAsk({ action: "prev" }));
  els.askSkipBtn.addEventListener("click", () => submitAsk({ action: "skip" }));
  els.askSubmitBtn.addEventListener("click", () => {
    submitAsk({ action: "submit", selected_indices: Array.from(state.selectedIndices) });
  });

  return {
    selectSession,
    openEngineManager,
    handleWsEvent,
    initDraftDefaults: (prov, model, needsOnboarding = false, readyProviders = []) => {
      state.readyProviders = Array.isArray(readyProviders) ? readyProviders : [];
      if (prov) state.draft.provider = prov;
      if (model) state.draft.model = model;
      state.needsOnboarding = Boolean(needsOnboarding);
    },
    setTelegramState: (connected, botUsername = "") => {
      state.telegramConnected = Boolean(connected);
      state.telegramBotUsername = botUsername || "";
      renderHeader();
      if (state.managingEngines || !state.session) renderMessages(false);
    },
    markHasExistingSessions: () => {
      if (state.needsOnboarding) {
        state.needsOnboarding = false;
        renderHeader();
        if (!state.session) renderMessages(false);
      }
    },
    onProviderSetupUpdated: async (prov) => {
      if (prov) delete state.modelCatalog[prov];
      await refreshReadyProviders();
      if (state.managingEngines) {
        wizardRenderer.renderInto(els.messages, { isWizard: false, isManager: true });
      } else if (!state.session && state.needsOnboarding) {
        wizardRenderer.renderInto(els.messages, { isWizard: true });
      }
    },
    refreshDraftTransportUi: () => {
      state.managingEngines = false;
      renderHeader();
      if (!state.session) renderMessages(false);
    },
    focusComposer: () => els.input?.focus(),
    getActiveSessionKey: () => state.session?.session_key || null
  };
}
