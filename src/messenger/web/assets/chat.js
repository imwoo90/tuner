import { escapeHtml, renderMarkdown } from "/dashboard/assets/markdown.js";
import { t, setLang, toggleLang, onLangChange } from "/dashboard/assets/i18n.js";
import {
  createSlashController,
  describeCommand,
  parseBuiltinSlashInput
} from "/dashboard/assets/slash.js";

export function createChatController({
  apiFetch,
  onOpenFile,
  onOpenInspectorTab,
  onRefreshSessions
}) {
  const els = {
    scrollWrap: document.getElementById("chat-scroll"),
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

  let state = {
    session: null,
    messages: [],
    liveStream: "",
    activeAsk: null,
    selectedIndices: new Set(),
    pendingFiles: [],
    modelCatalog: {},
    commandCatalog: {}
  };

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

  function renderMessages(forceScroll = false) {
    const stick = forceScroll || isNearBottom();
    els.messages.innerHTML = "";
    if (!state.session) {
      els.messages.innerHTML = `<div class="empty-state">${escapeHtml(t("emptyNoSession"))}</div>`;
      return;
    }
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
    if (!state.liveStream) {
      els.liveBubble.classList.add("hidden");
      els.liveBody.innerHTML = "";
      return;
    }
    const stick = isNearBottom();
    els.liveBubble.classList.remove("hidden");
    els.liveBody.innerHTML = renderMarkdown(state.liveStream);
    if (stick) scrollToBottom();
  }

  function renderAskCard() {
    const ask = state.activeAsk;
    if (!ask || !Array.isArray(ask.questions) || ask.questions.length === 0) {
      els.askCard.classList.add("hidden");
      els.input.placeholder = t("composerPlaceholder");
      return;
    }
    els.input.placeholder = t("composerAskPlaceholder");
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
    if (!state.session) return;
    const prov = state.session.provider || "antigravity";
    els.selProvider.value = prov;
    const cat = state.modelCatalog[prov] || [];
    els.selModel.innerHTML = "";
    let hasCurrent = false;
    for (const item of cat) {
      const opt = document.createElement("option");
      opt.value = item.model;
      opt.textContent = item.model;
      if (item.model === state.session.model) hasCurrent = true;
      els.selModel.appendChild(opt);
    }
    if (!hasCurrent && state.session.model) {
      const opt = document.createElement("option");
      opt.value = state.session.model;
      opt.textContent = state.session.model;
      els.selModel.appendChild(opt);
    }
    els.selModel.value = state.session.model || "";

    const modelEntry = cat.find((m) => m.model === els.selModel.value);
    const efforts = modelEntry ? modelEntry.efforts || [] : [];
    els.selEffort.innerHTML = "";
    if (!efforts.length) {
      els.selEffort.classList.add("hidden");
      els.selEffort.disabled = true;
    } else {
      els.selEffort.classList.remove("hidden");
      els.selEffort.disabled = false;
      for (const ef of efforts) {
        const opt = document.createElement("option");
        opt.value = ef;
        opt.textContent = ef;
        els.selEffort.appendChild(opt);
      }
      els.selEffort.value = state.session.effort || efforts[0];
    }
  }

  function formatTopicLabel(s) {
    if (s.topic_name) return s.topic_name;
    return s.transport === "web"
      ? t("webTopicFallback", { id: s.topic_id })
      : t("tgTopicFallback", { id: s.topic_id || s.chat_id });
  }

  function renderHeader() {
    const s = state.session;
    if (!s) {
      els.topicTitle.textContent = t("selectTopic");
      els.topicKey.textContent = "";
      els.topicState.dataset.state = "Idle";
      els.topicState.textContent = "Idle";
      els.topicState.className = "status-badge hidden";
      els.stopBtn.classList.add("hidden");
      return;
    }
    els.topicTitle.textContent = formatTopicLabel(s);
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
    state.session = session;
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
    if (!session) return;

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
    if (!state.session) return;
    if (cmd === "new" || cmd === "reset") return resetCurrentSession();
    if (cmd === "stop" || cmd === "abort") return stopCurrentSession();
    if (cmd === "provider") {
      const p = args.toLowerCase();
      if (p === "agy" || p === "antigravity") return updateConfig({ provider: "antigravity" });
      if (p === "claude" || p === "codex") return updateConfig({ provider: p });
      els.selProvider.focus();
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
    if (cmd === "help") {
      const prov = state.session.provider || "antigravity";
      const cmds = state.commandCatalog[prov] || [];
      const lines = cmds.map((c) => `- \`/${c.command}\` — ${describeCommand(c)}`).join("\n");
      appendLocalSystemNotice(`### ${t("helpHeader", { prov })}\n${lines}\n\n${t("helpTip")}`);
    }
  }

  const slashCtrl = createSlashController({
    inputEl: els.input,
    menuEl: els.slashMenu,
    slashBtnEl: els.slashBtn,
    getProvider: () => state.session?.provider || "antigravity",
    getCommands: () => state.commandCatalog[state.session?.provider || "antigravity"] || [],
    getModels: () => state.modelCatalog[state.session?.provider || "antigravity"] || [],
    onRunBuiltin: runBuiltinSlash
  });

  async function ensureActiveSession(seedText = "") {
    if (state.session) return state.session;
    const title = seedText.replace(/\s+/g, " ").trim().slice(0, 36) || null;
    const prov = els.selProvider?.value || "antigravity";
    const res = await apiFetch("/dashboard/api/sessions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ topic_name: title, provider: prov })
    });
    if (!res.ok) return null;
    const created = await res.json();
    await selectSession(created);
    onRefreshSessions?.();
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
    if (!state.session) return;
    const res = await apiFetch("/dashboard/api/sessions/config", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ session_key: state.session.session_key, ...patch })
    });
    if (res.ok) {
      const updated = await res.json();
      state.session = updated;
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

  els.selProvider.addEventListener("change", () => updateConfig({ provider: els.selProvider.value }));
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
    handleWsEvent,
    getActiveSessionKey: () => state.session?.session_key || null
  };
}
