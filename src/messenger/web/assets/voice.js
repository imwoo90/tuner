import { t } from "/dashboard/assets/i18n.js";
import { escapeHtml, renderMarkdown } from "/dashboard/assets/markdown.js";

const WORKLET_CODE = `
class Pcm16CaptureProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buf = new Int16Array(960);
    this.off = 0;
    this.readPos = 0;
    this.sumSq = 0;
  }
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (!ch || !ch.length) return true;
    const ratio = sampleRate / 16000;
    while (this.readPos < ch.length) {
      const start = Math.floor(this.readPos);
      const end = Math.min(ch.length, Math.max(start + 1, Math.floor(this.readPos + ratio)));
      let acc = 0;
      for (let k = start; k < end; k++) acc += ch[k];
      const s = Math.max(-1, Math.min(1, acc / (end - start)));
      this.sumSq += s * s;
      this.buf[this.off++] = s < 0 ? s * 0x8000 : s * 0x7fff;
      if (this.off >= this.buf.length) {
        const rms = Math.sqrt(this.sumSq / this.buf.length);
        const copy = this.buf.slice(0);
        this.port.postMessage({ pcm: copy.buffer, rms }, [copy.buffer]);
        this.off = 0;
        this.sumSq = 0;
      }
      this.readPos += ratio;
    }
    this.readPos -= ch.length;
    return true;
  }
}
registerProcessor("pcm16-capture", Pcm16CaptureProcessor);
`;

const SILENT_PCM960_B64 = (() => {
  let bin = "";
  for (let i = 0; i < 1920; i++) bin += "\x00";
  return btoa(bin);
})();

function bytesToBase64(bytes) {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) {
    bin += String.fromCharCode(bytes[i]);
  }
  return btoa(bin);
}

function base64ToInt16(b64) {
  const bin = atob(b64);
  const len = bin.length - (bin.length % 2);
  const bytes = new Uint8Array(len);
  for (let i = 0; i < len; i++) {
    bytes[i] = bin.charCodeAt(i);
  }
  return new Int16Array(bytes.buffer);
}

function cleanSnippet(raw) {
  return String(raw || "")
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/^[#>!*`\-\s]+/gm, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 110);
}

export function createVoiceController({
  apiFetch,
  getCsrfToken,
  getLang,
  getActiveSessionKey,
  getSessions,
  onFocusTopic,
  onNeedGeminiSetup
}) {
  const els = {
    chatPane: document.getElementById("chat-pane"),
    btn: document.getElementById("btn-voice"),
    bar: document.getElementById("voice-bar"),
    stageBody: document.getElementById("voice-stage-body"),
    wave: document.getElementById("voice-wave"),
    badge: document.getElementById("voice-status-badge"),
    sub: document.getElementById("voice-subtitle"),
    transcript: document.getElementById("voice-transcript"),
    deck: document.getElementById("voice-topic-deck"),
    textDrawer: document.getElementById("voice-text-drawer"),
    input: document.getElementById("voice-quick-input"),
    sendTextBtn: document.getElementById("btn-voice-send-text"),
    volSlider: document.getElementById("voice-volume"),
    muteBtn: document.getElementById("btn-voice-mute"),
    muteIcon: document.getElementById("voice-mute-icon"),
    muteLabel: document.getElementById("voice-mute-label"),
    kbdBtn: document.getElementById("btn-voice-kbd"),
    hangupBtn: document.getElementById("btn-voice-hangup"),
    peekSheet: document.getElementById("voice-peek-sheet"),
    peekTitle: document.getElementById("voice-peek-title"),
    peekState: document.getElementById("voice-peek-state"),
    peekBody: document.getElementById("voice-peek-body"),
    peekJumpBtn: document.getElementById("btn-voice-peek-jump"),
    peekCloseBtn: document.getElementById("btn-voice-peek-close")
  };

  let vws = null;
  let active = false;
  let muted = false;
  let kbdOpen = false;
  let peekSessionKey = "";
  let cachedCsrf = "";
  let reconnectTimer = null;
  let reconnectAttempts = 0;
  let listenDebounceTimer = null;
  let wakeLockSentinel = null;
  let audioCtx = null;
  let outGain = null;
  let mediaStream = null;
  let micSourceNode = null;
  let workletNode = null;
  let zeroGainNode = null;
  let workletLoaded = false;
  let nextPlayTime = 0;
  let aiSpeakingUntilMs = 0;
  let bargeHoldUntil = 0;
  let bargeStreak = 0;
  let pendingBargeFrame = null;
  const playingSources = new Set();

  let turns = [];
  const touchedKeys = new Set();
  const liveDeltas = new Map();
  const openAsks = new Map();
  const recentResults = new Map();
  const fetchingAskKeys = new Set();

  const savedVol = Number(localStorage.getItem("tuner_voice_vol"));
  let currentGain = Number.isFinite(savedVol) && savedVol >= 0 && savedVol <= 200 ? savedVol / 100 : 1.0;
  if (els.volSlider) {
    els.volSlider.value = `${Math.round(currentGain * 100)}`;
  }

  window.__voiceAudioChunksReceived = 0;

  function setStatus(stateKey, mode = "listening") {
    if (els.badge) {
      els.badge.textContent = t(stateKey);
      els.badge.className = `pill voice-status-pill ${mode === "thinking" ? "pill-warn" : (muted ? "" : "pill-ok")}`;
    }
    if (els.wave) {
      els.wave.dataset.mode = muted ? "muted" : mode;
      if (muted || mode !== "listening") {
        els.wave.style.setProperty("--mic-scale", "1");
      }
    }
  }

  function closeOpenTurns() {
    for (const item of turns) item.open = false;
  }

  function renderTranscript() {
    if (!els.transcript || !els.stageBody) return;
    els.stageBody.classList.toggle("has-turns", turns.length > 0);
    els.transcript.innerHTML = turns
      .map((item) => {
        if (item.role === "tool") {
          return `<div class="voice-cap-tool">⚡ ${escapeHtml(item.text)}</div>`;
        }
        const isUser = item.role === "user";
        const speaker = isUser ? `🗣️ ${t("voiceYouLabel")}` : `🤖 ${t("voiceBotLabel")}`;
        return `<div class="voice-cap-row ${isUser ? "voice-cap-user" : "voice-cap-bot"}">
          <span class="voice-cap-speaker">${escapeHtml(speaker)}</span>
          <div class="voice-cap-bubble">${escapeHtml(item.text.trim())}</div>
        </div>`;
      })
      .join("");
    els.transcript.scrollTop = els.transcript.scrollHeight;
  }

  function appendCaption(role, text, replaceIfNew = false) {
    const chunk = String(text || "");
    if (!chunk.trim()) return;
    if (role === "tool") {
      closeOpenTurns();
      turns.push({ role: "tool", text: chunk.trim(), open: false });
      renderTranscript();
      return;
    }
    const last = turns[turns.length - 1];
    if (!replaceIfNew && last && last.role === role && last.open) {
      last.text += chunk;
    } else {
      closeOpenTurns();
      turns.push({ role, text: chunk, open: !replaceIfNew });
    }
    if (role === "bot" && els.sub) {
      els.sub.textContent = turns[turns.length - 1].text.trim();
    }
    renderTranscript();
  }

  function formatTopicName(s) {
    if (!s) return "";
    if (s.topic_name) return s.topic_name;
    return s.transport === "web"
      ? t("webTopicFallback", { id: s.topic_id })
      : t("tgTopicFallback", { id: s.topic_id || s.chat_id });
  }

  async function hydrateOpenAsk(sk) {
    if (!sk || openAsks.has(sk) || fetchingAskKeys.has(sk)) return;
    fetchingAskKeys.add(sk);
    try {
      const r = await apiFetch(`/dashboard/api/sessions/history?session_key=${encodeURIComponent(sk)}`);
      if (!r.ok) return;
      const d = await r.json();
      if (d.active_ask && Array.isArray(d.active_ask.questions) && d.active_ask.questions.length > 0) {
        openAsks.set(sk, d.active_ask);
        if (active) renderDeck();
      }
    } catch (_) {
    } finally {
      fetchingAskKeys.delete(sk);
    }
  }

  function renderDeck() {
    if (!els.deck || !active) return;
    const all = (typeof getSessions === "function" ? getSessions() : []) || [];
    for (const s of all) {
      if (s.asking && !openAsks.has(s.session_key)) {
        hydrateOpenAsk(s.session_key);
      }
    }
    const candidates = all.filter(
      (s) => s.asking || s.busy || openAsks.has(s.session_key) || touchedKeys.has(s.session_key) || recentResults.has(s.session_key)
    );
    candidates.sort((a, b) => {
      const score = (x) => (x.asking || openAsks.has(x.session_key) ? 3 : x.busy ? 2 : 1);
      return score(b) - score(a);
    });
    const top = candidates.slice(0, 3);
    if (top.length === 0) {
      els.deck.classList.add("hidden");
      els.deck.innerHTML = "";
      return;
    }
    els.deck.classList.remove("hidden");
    let html = `<div class="voice-deck-label">${escapeHtml(t("voiceDeckHeader"))}</div>`;
    for (const s of top) {
      const sk = s.session_key;
      const ask = openAsks.get(sk);
      const isAsking = Boolean(s.asking || ask);
      const isBusy = Boolean(s.busy && !isAsking);
      const badgeCls = isAsking ? "pill-warn" : (isBusy ? "pill-ok" : "");
      const badgeTxt = isAsking ? `❓ ${t("voiceAskingBadge")}` : (isBusy ? `⚡ ${t("voiceRunningBadge")}` : `✅ ${t("voiceDoneBadge")}`);
      const chanIcon = s.transport === "tg" ? "✈️" : "🌐";

      let bodyHtml = "";
      if (isAsking && ask && Array.isArray(ask.questions)) {
        const qIdx = Number(ask.current_index || 0);
        const qObj = ask.questions[qIdx] || ask.questions[0];
        if (qObj) {
          const opts = (qObj.options || [])
            .map((opt, idx) => `<button type="button" class="btn btn-sm voice-ask-opt" data-sk="${escapeHtml(sk)}" data-idx="${idx}" data-label="${escapeHtml(opt)}">${idx + 1}. ${escapeHtml(opt)}</button>`)
            .join("");
          bodyHtml = `<div class="voice-deck-ask-q">(${qIdx + 1}/${ask.questions.length}) ${escapeHtml(qObj.question || "")}</div>
            <div class="voice-deck-ask-opts">${opts}</div>`;
        }
      } else {
        const rawSnip = liveDeltas.get(sk) || recentResults.get(sk)?.text || "";
        const snip = cleanSnippet(rawSnip);
        if (snip) {
          bodyHtml = `<div class="voice-deck-snippet">${escapeHtml(snip)}</div>`;
        }
      }

      html += `<div class="voice-deck-card${isAsking ? " is-asking" : ""}">
        <div class="voice-deck-top">
          <div class="voice-deck-title">
            <span>${chanIcon}</span>
            <span class="voice-deck-title-txt">${escapeHtml(formatTopicName(s))}</span>
            <span class="pill ${badgeCls}">${escapeHtml(badgeTxt)}</span>
          </div>
          <div class="card-row">
            ${isBusy ? `<button type="button" class="btn btn-sm voice-card-stop" data-sk="${escapeHtml(sk)}">${escapeHtml(t("voiceStopBtn"))}</button>` : ""}
            <button type="button" class="btn btn-sm voice-card-peek" data-sk="${escapeHtml(sk)}">${escapeHtml(t("voicePeekBtn"))}</button>
          </div>
        </div>
        ${bodyHtml}
      </div>`;
    }
    els.deck.innerHTML = html;

    els.deck.querySelectorAll(".voice-ask-opt").forEach((b) => {
      b.addEventListener("click", async () => {
        b.disabled = true;
        const sk = b.dataset.sk;
        const idx = Number(b.dataset.idx || 0);
        await apiFetch("/dashboard/api/sessions/ask", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            session_key: sk,
            action: "answer",
            selected_indices: [idx],
            answer: b.dataset.label || ""
          })
        });
      });
    });

    els.deck.querySelectorAll(".voice-card-stop").forEach((b) => {
      b.addEventListener("click", async () => {
        b.disabled = true;
        await apiFetch("/dashboard/api/sessions/stop", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ session_key: b.dataset.sk })
        });
      });
    });

    els.deck.querySelectorAll(".voice-card-peek").forEach((b) => {
      b.addEventListener("click", () => openPeek(b.dataset.sk));
    });
  }

  async function openPeek(sk) {
    if (!sk || !els.peekSheet || !els.peekBody) return;
    peekSessionKey = sk;
    const all = (typeof getSessions === "function" ? getSessions() : []) || [];
    const s = all.find((x) => x.session_key === sk);
    if (els.peekTitle) els.peekTitle.textContent = s ? formatTopicName(s) : sk;
    if (els.peekState) {
      els.peekState.textContent = s?.asking ? t("voiceAskingBadge") : (s?.busy ? t("voiceRunningBadge") : "Idle");
    }
    els.peekSheet.classList.remove("hidden");
    els.peekBody.innerHTML = `<div class="muted">…</div>`;
    const r = await apiFetch(`/dashboard/api/sessions/history?session_key=${encodeURIComponent(sk)}`);
    if (!r.ok || peekSessionKey !== sk) return;
    const d = await r.json();
    const msgs = d.messages || [];
    let html = "";
    for (const m of msgs.slice(-20)) {
      const isUser = m.sender === "user";
      html += `<div class="msg-bubble ${isUser ? "msg-user" : "msg-bot"}">
        <div class="msg-body">${renderMarkdown(m.text || "")}</div>
      </div>`;
    }
    if (d.live_stream) {
      html += `<div class="live-stream"><div class="msg-body">${renderMarkdown(d.live_stream)}</div></div>`;
    }
    if (!html) {
      html = `<div class="muted">${escapeHtml(t("voicePeekEmpty"))}</div>`;
    }
    els.peekBody.innerHTML = html;
    els.peekBody.scrollTop = els.peekBody.scrollHeight;
  }

  function closePeek() {
    peekSessionKey = "";
    els.peekSheet?.classList.add("hidden");
  }

  function flushPlayback() {
    if (listenDebounceTimer) {
      clearTimeout(listenDebounceTimer);
      listenDebounceTimer = null;
    }
    for (const src of playingSources) {
      try { src.stop(); } catch (_) {}
    }
    playingSources.clear();
    aiSpeakingUntilMs = 0;
    bargeStreak = 0;
    pendingBargeFrame = null;
    if (audioCtx) nextPlayTime = audioCtx.currentTime;
  }

  async function acquireWakeLock() {
    if (!active || wakeLockSentinel || !navigator.wakeLock?.request) return;
    try {
      wakeLockSentinel = await navigator.wakeLock.request("screen");
      wakeLockSentinel.addEventListener("release", () => {
        wakeLockSentinel = null;
      });
    } catch (_) {}
  }

  function releaseWakeLock() {
    if (!wakeLockSentinel) return;
    try { wakeLockSentinel.release(); } catch (_) {}
    wakeLockSentinel = null;
  }

  function unlockAudioGestureSync() {
    if (!audioCtx) {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      if (Ctx) {
        audioCtx = new Ctx({ latencyHint: "interactive" });
        outGain = audioCtx.createGain();
        outGain.gain.value = currentGain;
        outGain.connect(audioCtx.destination);
        workletLoaded = false;
      }
    }
    if (audioCtx && audioCtx.state === "suspended") {
      audioCtx.resume().catch(() => {});
    }
    // Fire a 1-frame silent buffer inside the synchronous click handler to unlock mobile iOS/Android playback!
    if (audioCtx && outGain) {
      try {
        const silent = audioCtx.createBuffer(1, 1, 24000);
        const s = audioCtx.createBufferSource();
        s.buffer = silent;
        s.connect(outGain);
        s.start(0);
      } catch (_) {}
    }
    return audioCtx;
  }

  function enqueuePcm24(b64) {
    window.__voiceAudioChunksReceived = (window.__voiceAudioChunksReceived || 0) + 1;
    const ctx = unlockAudioGestureSync();
    if (!ctx || !outGain) return;
    let pcm;
    try {
      pcm = base64ToInt16(b64);
    } catch (_) {
      return;
    }
    if (!pcm.length) return;
    if (listenDebounceTimer) {
      clearTimeout(listenDebounceTimer);
      listenDebounceTimer = null;
    }
    const buf = ctx.createBuffer(1, pcm.length, 24000);
    const ch = buf.getChannelData(0);
    for (let i = 0; i < pcm.length; i++) {
      ch[i] = pcm[i] / 32768;
    }
    const src = ctx.createBufferSource();
    src.buffer = buf;
    src.connect(outGain);
    // Use a 90ms initial jitter cushion when starting a fresh utterance so LTE/Tunnel packet bursts play gaplessly!
    const isFreshUtterance = playingSources.size === 0 || nextPlayTime <= ctx.currentTime + 0.005;
    const startAt = isFreshUtterance ? ctx.currentTime + 0.09 : nextPlayTime;
    nextPlayTime = startAt + buf.duration;
    const remMs = Math.max(0, (nextPlayTime - ctx.currentTime) * 1000);
    aiSpeakingUntilMs = Math.max(aiSpeakingUntilMs, performance.now() + remMs + 340);
    playingSources.add(src);
    setStatus("voiceSpeaking", "speaking");
    src.onended = () => {
      playingSources.delete(src);
      if (playingSources.size === 0 && active) {
        if (listenDebounceTimer) clearTimeout(listenDebounceTimer);
        listenDebounceTimer = setTimeout(() => {
          listenDebounceTimer = null;
          if (playingSources.size === 0 && active) {
            setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
          }
        }, 180);
      }
    };
    try { src.start(startAt); } catch (_) { playingSources.delete(src); }
  }

  async function startMicCapture(initialStream) {
    if (mediaStream && workletNode) return;
    if (!window.isSecureContext && !navigator.mediaDevices?.getUserMedia) {
      if (active) appendCaption("tool", `⚠️ ${t("voiceInsecureOriginWarn")}`, true);
      return;
    }
    const stream = initialStream || (navigator.mediaDevices?.getUserMedia
      ? await navigator.mediaDevices.getUserMedia({
          audio: {
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true,
            channelCount: 1
          }
        }).catch(() => null)
      : null);
    if (!stream) {
      const warnKey = window.isSecureContext ? "voiceMicDeniedWarn" : "voiceInsecureOriginWarn";
      if (active) appendCaption("tool", `⚠️ ${t(warnKey)}`, true);
      return;
    }
    if (!active) {
      stream.getTracks().forEach((tr) => tr.stop());
      return;
    }
    mediaStream = stream;
    const ctx = unlockAudioGestureSync();
    if (!ctx || !ctx.audioWorklet) return;
    try {
      if (!workletLoaded) {
        const blob = new Blob([WORKLET_CODE], { type: "application/javascript" });
        const modUrl = URL.createObjectURL(blob);
        try {
          await ctx.audioWorklet.addModule(modUrl);
          workletLoaded = true;
        } finally {
          URL.revokeObjectURL(modUrl);
        }
      }
      if (!active) return;
      if (ctx.state === "suspended") {
        await ctx.resume().catch(() => {});
      }
      // Keep micSourceNode -> workletNode -> zeroGainNode -> ctx.destination referenced in closure
      // so V8 GC never collects MediaStreamAudioSourceNode and Blink continuously pulls process()!
      micSourceNode = ctx.createMediaStreamSource(mediaStream);
      workletNode = new AudioWorkletNode(ctx, "pcm16-capture", {
        numberOfInputs: 1,
        numberOfOutputs: 1,
        outputChannelCount: [1]
      });
      zeroGainNode = ctx.createGain();
      zeroGainNode.gain.value = 0;

      workletNode.port.onmessage = (ev) => {
        if (!active || muted || !vws || vws.readyState !== WebSocket.OPEN) return;
        const { pcm, rms } = ev.data || {};
        if (!pcm) return;
        const nowMs = performance.now();
        const b64 = bytesToBase64(new Uint8Array(pcm));
        const inSpeakerWindow = playingSources.size > 0 || nowMs < aiSpeakingUntilMs;
        if (!inSpeakerWindow && els.wave) {
          const scale = (1 + Math.min(0.26, (rms || 0) * 2.8)).toFixed(3);
          els.wave.style.setProperty("--mic-scale", scale);
        }
        // Speakerphone residual-echo & transient-pop guard: while AI audio is playing (plus 340ms acoustic room tail),
        // require 2 consecutive 60ms frames (120ms) >= 0.045 RMS before opening a 1.4s barge-in pass-through window,
        // flushing the buffered 1st syllable frame so genuine human interruption never clips!
        if (inSpeakerWindow) {
          if (nowMs <= bargeHoldUntil) {
            if ((rms || 0) >= 0.045) bargeHoldUntil = nowMs + 1400;
            vws.send(JSON.stringify({ type: "audio", data: b64 }));
            return;
          }
          if ((rms || 0) >= 0.045) {
            bargeStreak += 1;
            if (bargeStreak >= 2) {
              bargeHoldUntil = nowMs + 1400;
              bargeStreak = 0;
              if (pendingBargeFrame) {
                vws.send(JSON.stringify({ type: "audio", data: pendingBargeFrame }));
                pendingBargeFrame = null;
              }
              vws.send(JSON.stringify({ type: "audio", data: b64 }));
              return;
            }
            pendingBargeFrame = b64;
          } else {
            bargeStreak = 0;
            pendingBargeFrame = null;
          }
          vws.send(JSON.stringify({ type: "audio", data: SILENT_PCM960_B64 }));
          return;
        }
        bargeStreak = 0;
        pendingBargeFrame = null;
        vws.send(JSON.stringify({ type: "audio", data: b64 }));
      };

      micSourceNode.connect(workletNode);
      workletNode.connect(zeroGainNode);
      zeroGainNode.connect(ctx.destination);
    } catch (err) {
      if (active) appendCaption("tool", `⚠️ Mic error: ${err?.message || err}`, true);
    }
  }

  function stopHardware() {
    flushPlayback();
    if (micSourceNode) {
      try { micSourceNode.disconnect(); } catch (_) {}
      micSourceNode = null;
    }
    if (workletNode) {
      try { workletNode.disconnect(); } catch (_) {}
      workletNode = null;
    }
    if (zeroGainNode) {
      try { zeroGainNode.disconnect(); } catch (_) {}
      zeroGainNode = null;
    }
    if (mediaStream) {
      mediaStream.getTracks().forEach((tr) => tr.stop());
      mediaStream = null;
    }
  }

  function syncButtons() {
    els.chatPane?.classList.toggle("voice-stage-open", active);
    els.btn?.classList.toggle("active", active);
    els.bar?.classList.toggle("hidden", !active);
    els.textDrawer?.classList.toggle("hidden", !active || !kbdOpen);
    els.kbdBtn?.classList.toggle("active", active && kbdOpen);
    if (els.muteBtn) {
      els.muteBtn.title = muted ? t("voiceUnmuteTitle") : t("voiceMuteTitle");
      els.muteBtn.classList.toggle("muted", muted);
    }
    if (els.muteIcon) els.muteIcon.textContent = muted ? "🔇" : "🎤";
    if (els.muteLabel) els.muteLabel.textContent = t(muted ? "voiceMicMutedBtn" : "voiceMicOnBtn");
  }

  function stopCall() {
    active = false;
    muted = false;
    kbdOpen = false;
    reconnectAttempts = 0;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    releaseWakeLock();
    closePeek();
    if (vws) {
      const sock = vws;
      vws = null;
      try { sock.close(); } catch (_) {}
    }
    stopHardware();
    syncButtons();
  }

  function openVoiceSocket(pendingMicPromise) {
    if (!active) return;
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    const sk = encodeURIComponent(getActiveSessionKey() || "");
    const lang = encodeURIComponent(getLang() || "ko");
    const csrf = encodeURIComponent(getCsrfToken() || cachedCsrf || "");
    const sock = new WebSocket(`${proto}//${window.location.host}/dashboard/api/voice/ws?csrf=${csrf}&lang=${lang}&topic=${sk}`);
    vws = sock;

    sock.addEventListener("open", async () => {
      if (vws !== sock || !active) return;
      const mic = pendingMicPromise ? await pendingMicPromise : null;
      await startMicCapture(mic);
    });

    sock.addEventListener("message", (ev) => {
      if (vws !== sock || !active) return;
      let msg;
      try { msg = JSON.parse(ev.data); } catch (_) { return; }
      if (msg.type === "ready") {
        reconnectAttempts = 0;
        setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
        if (turns.length === 0 && els.sub) els.sub.textContent = t("voiceReadyHint");
      } else if (msg.type === "interrupted") {
        flushPlayback();
        closeOpenTurns();
        setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
      } else if (msg.type === "audio" && msg.data) {
        enqueuePcm24(msg.data);
      } else if (msg.type === "user_transcript" && msg.text) {
        appendCaption("user", msg.text, false);
      } else if (msg.type === "bot_transcript" && msg.text) {
        appendCaption("bot", msg.text, false);
      } else if (msg.type === "tool_call" && msg.name) {
        setStatus("voiceThinking", "thinking");
        appendCaption("tool", msg.name, true);
      } else if (msg.type === "focus_topic" && msg.session_key) {
        touchedKeys.add(msg.session_key);
        onFocusTopic?.(msg.session_key);
        renderDeck();
      } else if (msg.type === "turn_complete") {
        closeOpenTurns();
        if (playingSources.size === 0) {
          setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
        }
      } else if (msg.type === "error") {
        appendCaption("tool", `⚠️ ${msg.error || "Voice error"}`, true);
      }
    });

    sock.addEventListener("close", () => {
      if (vws !== sock || !active) return;
      vws = null;
      if (reconnectAttempts < 4) {
        reconnectAttempts += 1;
        setStatus("voiceReconnecting", "thinking");
        const delay = Math.min(3500, 650 * reconnectAttempts);
        reconnectTimer = setTimeout(() => {
          reconnectTimer = null;
          openVoiceSocket(null);
        }, delay);
      } else {
        stopCall();
      }
    });
  }

  async function startCall() {
    // 1. Unlock AudioContext + request getUserMedia IMMEDIATELY inside the synchronous user click gesture!
    unlockAudioGestureSync();
    const pendingMicPromise = navigator.mediaDevices?.getUserMedia
      ? navigator.mediaDevices.getUserMedia({
          audio: {
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true,
            channelCount: 1
          }
        }).catch(() => null)
      : Promise.resolve(null);

    const stRes = await apiFetch("/dashboard/api/voice/status");
    if (!stRes.ok) {
      pendingMicPromise.then((s) => s?.getTracks().forEach((tr) => tr.stop()));
      return;
    }
    const st = await stRes.json();
    if (st.csrf_token) cachedCsrf = st.csrf_token;
    if (!st.ready) {
      pendingMicPromise.then((s) => s?.getTracks().forEach((tr) => tr.stop()));
      onNeedGeminiSetup?.();
      return;
    }

    active = true;
    muted = false;
    kbdOpen = false;
    reconnectAttempts = 0;
    turns = [];
    touchedKeys.clear();
    liveDeltas.clear();
    openAsks.clear();
    recentResults.clear();
    closePeek();
    window.__voiceAudioChunksReceived = 0;
    acquireWakeLock();
    renderTranscript();
    syncButtons();
    renderDeck();
    setStatus("voiceConnecting", "thinking");
    if (els.sub) els.sub.textContent = t("voiceConnectingHint");
    openVoiceSocket(pendingMicPromise);
  }

  function sendText(text) {
    const clean = (text || "").trim();
    if (!clean || !vws || vws.readyState !== WebSocket.OPEN) return false;
    closeOpenTurns();
    appendCaption("user", clean, true);
    setStatus("voiceThinking", "thinking");
    vws.send(JSON.stringify({ type: "text", text: clean }));
    return true;
  }

  function submitTextDrawer() {
    if (!els.input) return;
    if (sendText(els.input.value)) {
      els.input.value = "";
    }
  }

  els.btn?.addEventListener("click", () => {
    if (active) stopCall();
    else startCall();
  });

  // Tap Audio Orb while assistant is speaking to instantly interrupt playback (manual 0ms Barge-In)!
  els.wave?.addEventListener("click", () => {
    if (active && playingSources.size > 0) {
      flushPlayback();
      closeOpenTurns();
      setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
    }
  });

  els.muteBtn?.addEventListener("click", () => {
    if (!active) return;
    muted = !muted;
    syncButtons();
    if (playingSources.size === 0) {
      setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
    }
  });

  els.kbdBtn?.addEventListener("click", () => {
    if (!active) return;
    kbdOpen = !kbdOpen;
    syncButtons();
    if (kbdOpen) setTimeout(() => els.input?.focus(), 20);
  });

  els.sendTextBtn?.addEventListener("click", submitTextDrawer);

  els.hangupBtn?.addEventListener("click", stopCall);

  els.volSlider?.addEventListener("input", () => {
    const v = Math.max(0, Math.min(200, Number(els.volSlider.value) || 0));
    currentGain = v / 100;
    localStorage.setItem("tuner_voice_vol", `${v}`);
    if (outGain) outGain.gain.value = currentGain;
  });

  els.input?.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submitTextDrawer();
    }
  });

  document.addEventListener("visibilitychange", () => {
    if (active && document.visibilityState === "visible") {
      acquireWakeLock();
      if (audioCtx && audioCtx.state === "suspended") {
        audioCtx.resume().catch(() => {});
      }
    }
  });

  els.peekCloseBtn?.addEventListener("click", closePeek);
  els.peekJumpBtn?.addEventListener("click", async () => {
    const sk = peekSessionKey;
    closePeek();
    stopCall();
    if (sk) await onFocusTopic?.(sk);
  });

  return {
    isActive: () => active,
    sendText,
    refreshDeck: () => {
      if (active) renderDeck();
    },
    handleWsEvent: (ev) => {
      if (!active || !ev) return;
      if (ev.type === "session_updated") {
        if (ev.busy || ev.asking) touchedKeys.add(ev.session_key);
        if (!ev.asking) openAsks.delete(ev.session_key);
        renderDeck();
      } else if (ev.type === "stream_delta" && ev.session_key) {
        touchedKeys.add(ev.session_key);
        liveDeltas.set(ev.session_key, ev.text || "");
        renderDeck();
        if (peekSessionKey === ev.session_key) openPeek(ev.session_key);
      } else if (ev.type === "ask_question" && ev.session_key) {
        touchedKeys.add(ev.session_key);
        openAsks.set(ev.session_key, ev);
        renderDeck();
      } else if (ev.type === "ask_resolved" && ev.session_key) {
        openAsks.delete(ev.session_key);
        renderDeck();
      } else if (ev.type === "turn_result" && ev.session_key) {
        touchedKeys.add(ev.session_key);
        liveDeltas.delete(ev.session_key);
        recentResults.set(ev.session_key, { text: ev.text || "", isError: Boolean(ev.is_error) });
        renderDeck();
        if (peekSessionKey === ev.session_key) openPeek(ev.session_key);
      }
    },
    notifyActiveTopic: (sk) => {
      if (active && vws && vws.readyState === WebSocket.OPEN) {
        vws.send(JSON.stringify({ type: "active_topic", session_key: sk || "" }));
      }
    },
    refreshI18n: () => {
      if (active) {
        syncButtons();
        renderTranscript();
        renderDeck();
      }
    }
  };
}
