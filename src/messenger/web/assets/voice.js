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
    cfgBtn: document.getElementById("btn-voice-audio-cfg"),
    cfgSummary: document.getElementById("voice-cfg-summary"),
    cfgPanel: document.getElementById("voice-cfg-panel"),
    volPresets: document.getElementById("voice-vol-presets"),
    langModes: document.getElementById("voice-lang-modes"),
    micModes: document.getElementById("voice-mic-modes"),
    micModeDesc: document.getElementById("voice-mic-mode-desc"),
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
  let cfgOpen = false;
  let pttTalking = false;
  let pttDownAt = 0;
  let peekSessionKey = "";
  let cachedCsrf = "";
  let reconnectTimer = null;
  let reconnectAttempts = 0;
  let listenDebounceTimer = null;
  let wakeLockSentinel = null;
  let audioCtx = null;
  let outGain = null;
  let outLimiter = null;
  let mediaStream = null;
  let micSourceNode = null;
  let workletNode = null;
  let zeroGainNode = null;
  let workletLoaded = false;
  let nextPlayTime = 0;
  let aiSpeakingUntilMs = 0;
  let speechNormGain = 1.0;
  let bargeHoldUntil = 0;
  let bargeStreak = 0;
  let pendingBargeFrames = [];
  let noiseFloor = 0.002;
  let speechOpenUntilMs = 0;
  let voiceOnsetStreak = 0;
  let preRollFrames = [];
  const playingSources = new Set();

  let turns = [];
  const touchedKeys = new Set();
  const liveDeltas = new Map();
  const openAsks = new Map();
  const recentResults = new Map();
  const fetchingAskKeys = new Set();

  const VOL_PRESETS = [60, 100, 135, 170];
  const rawSavedVol = Number(localStorage.getItem("tuner_voice_vol"));
  let currentVolPct = Number.isFinite(rawSavedVol) && rawSavedVol > 0
    ? VOL_PRESETS.reduce((best, p) => (Math.abs(p - rawSavedVol) < Math.abs(best - rawSavedVol) ? p : best), 100)
    : 100;
  let currentGain = currentVolPct / 100;

  const savedMicMode = localStorage.getItem("tuner_voice_mic_mode");
  let micMode = savedMicMode === "strict" || savedMicMode === "ptt" ? savedMicMode : "near";

  const savedVoiceLang = localStorage.getItem("tuner_voice_lang");
  let voiceLang = savedVoiceLang === "ko" || savedVoiceLang === "en" || savedVoiceLang === "auto"
    ? savedVoiceLang
    : "";

  function effectiveVoiceLang() {
    if (voiceLang === "ko" || voiceLang === "en" || voiceLang === "auto") return voiceLang;
    return String(getLang?.() || "ko").startsWith("en") ? "en" : "ko";
  }

  window.__voiceAudioChunksReceived = 0;

  function micConstraints() {
    return {
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: false,
        voiceIsolation: true,
        channelCount: 1
      }
    };
  }

  function idleStatePair() {
    if (micMode === "ptt") {
      return pttTalking ? ["voiceListening", "listening"] : ["voicePttReadyBadge", "muted"];
    }
    return muted ? ["voiceMuted", "muted"] : ["voiceListening", "listening"];
  }

  function setStatus(stateKey, mode = "listening") {
    const isMutedLook = mode === "muted" || (micMode !== "ptt" && muted);
    if (els.badge) {
      els.badge.textContent = t(stateKey);
      els.badge.className = `pill voice-status-pill ${mode === "thinking" ? "pill-warn" : (isMutedLook ? "" : "pill-ok")}`;
    }
    if (els.wave) {
      els.wave.dataset.mode = isMutedLook ? "muted" : mode;
      if (isMutedLook || mode !== "listening") {
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
    pendingBargeFrames = [];
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
        try {
          audioCtx = new Ctx({ latencyHint: "interactive", sampleRate: 48000 });
        } catch (_) {
          audioCtx = new Ctx({ latencyHint: "interactive" });
        }
        outGain = audioCtx.createGain();
        outGain.gain.value = currentGain;
        outLimiter = audioCtx.createDynamicsCompressor();
        outLimiter.threshold.value = -10;
        outLimiter.knee.value = 8;
        outLimiter.ratio.value = 12;
        outLimiter.attack.value = 0.002;
        outLimiter.release.value = 0.12;
        outGain.connect(outLimiter);
        outLimiter.connect(audioCtx.destination);
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
    // Per-chunk Automatic Loudness Leveler: prevents Gemini Live native audio from suddenly spiking +12~16 dB mid-call!
    let sumSq = 0;
    for (let i = 0; i < pcm.length; i++) {
      const x = pcm[i] / 32768;
      sumSq += x * x;
    }
    const chunkRms = Math.sqrt(sumSq / pcm.length);
    if (chunkRms > 0.018) {
      const target = Math.max(0.28, Math.min(1.18, 0.115 / chunkRms));
      const alpha = target < speechNormGain ? 0.48 : 0.10;
      speechNormGain = speechNormGain * (1 - alpha) + target * alpha;
    }
    const buf = ctx.createBuffer(1, pcm.length, 24000);
    const ch = buf.getChannelData(0);
    for (let i = 0; i < pcm.length; i++) {
      const scaled = (pcm[i] / 32768) * speechNormGain;
      ch[i] = scaled > 0.65 || scaled < -0.65 ? Math.tanh(scaled) * 0.72 : scaled;
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
            const [stKey, stMode] = idleStatePair();
            setStatus(stKey, stMode);
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
      ? await navigator.mediaDevices.getUserMedia(micConstraints()).catch(() => null)
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
        if (!active || !vws || vws.readyState !== WebSocket.OPEN) return;
        const { pcm, rms } = ev.data || {};
        if (!pcm) return;
        const valRms = rms || 0;
        const nowMs = performance.now();
        const b64 = bytesToBase64(new Uint8Array(pcm));

        // 1. Push-to-Talk (PTT) mode: 100% hard gate unless user is actively pressing/toggled [Talk]
        if (micMode === "ptt") {
          if (pttTalking) {
            if (els.wave) {
              els.wave.style.setProperty("--mic-scale", (1 + Math.min(0.26, valRms * 2.8)).toFixed(3));
            }
            vws.send(JSON.stringify({ type: "audio", data: b64 }));
          } else {
            vws.send(JSON.stringify({ type: "audio", data: SILENT_PCM960_B64 }));
          }
          return;
        }

        if (muted) return;

        // 2. Speakerphone acoustic echo & distant-voice guard while AI is speaking (+340ms room tail)
        const inSpeakerWindow = playingSources.size > 0 || nowMs < aiSpeakingUntilMs;
        if (inSpeakerWindow) {
          const bargeRms = micMode === "strict" ? 0.085 : 0.065;
          if (nowMs <= bargeHoldUntil) {
            if (valRms >= bargeRms) bargeHoldUntil = nowMs + 1400;
            vws.send(JSON.stringify({ type: "audio", data: b64 }));
            return;
          }
          if (valRms >= bargeRms) {
            bargeStreak += 1;
            pendingBargeFrames.push(b64);
            if (bargeStreak >= 3) {
              bargeHoldUntil = nowMs + 1400;
              bargeStreak = 0;
              for (const pf of pendingBargeFrames) {
                vws.send(JSON.stringify({ type: "audio", data: pf }));
              }
              pendingBargeFrames = [];
              return;
            }
          } else {
            bargeStreak = 0;
            pendingBargeFrames = [];
          }
          vws.send(JSON.stringify({ type: "audio", data: SILENT_PCM960_B64 }));
          return;
        }
        bargeStreak = 0;
        pendingBargeFrames = [];

        // 3. Adaptive Near-Field Foreground Voice Gate (blocks distant room chatter / TV murmurs)
        const minGateRms = micMode === "strict" ? 0.030 : 0.013;
        const snrMult = micMode === "strict" ? 3.2 : 2.6;
        const gateThr = Math.max(minGateRms, noiseFloor * snrMult);
        const holdMs = micMode === "strict" ? 650 : 780;

        if (nowMs <= speechOpenUntilMs) {
          if (valRms >= gateThr * 0.72) {
            speechOpenUntilMs = nowMs + holdMs;
          }
          if (els.wave) {
            els.wave.style.setProperty("--mic-scale", (1 + Math.min(0.26, valRms * 2.8)).toFixed(3));
          }
          vws.send(JSON.stringify({ type: "audio", data: b64 }));
          return;
        }

        if (valRms >= gateThr) {
          voiceOnsetStreak += 1;
          if (voiceOnsetStreak >= 2) {
            speechOpenUntilMs = nowMs + holdMs;
            voiceOnsetStreak = 0;
            for (const pre of preRollFrames) {
              vws.send(JSON.stringify({ type: "audio", data: pre }));
            }
            preRollFrames = [];
            if (els.wave) {
              els.wave.style.setProperty("--mic-scale", (1 + Math.min(0.26, valRms * 2.8)).toFixed(3));
            }
            vws.send(JSON.stringify({ type: "audio", data: b64 }));
            return;
          }
        } else {
          voiceOnsetStreak = 0;
          if (valRms < 0.035) {
            noiseFloor = valRms < noiseFloor
              ? noiseFloor * 0.85 + valRms * 0.15
              : Math.min(0.018, noiseFloor * 0.985 + valRms * 0.015);
          }
        }

        preRollFrames.push(b64);
        if (preRollFrames.length > 2) preRollFrames.shift();
        if (els.wave) els.wave.style.setProperty("--mic-scale", "1");
        vws.send(JSON.stringify({ type: "audio", data: SILENT_PCM960_B64 }));
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

  function renderCfgUi() {
    const effLang = effectiveVoiceLang();
    const langTag = effLang === "auto" ? "AUTO" : effLang.toUpperCase();
    const modeIcon = micMode === "strict" ? "🛡️" : (micMode === "ptt" ? "🎙️" : (currentVolPct <= 60 ? "🔈" : (currentVolPct >= 170 ? "📢" : "🔊")));
    if (els.cfgSummary) {
      els.cfgSummary.textContent = `${modeIcon} ${currentVolPct}% · ${langTag}`;
    }
    els.cfgBtn?.classList.toggle("active", cfgOpen);
    els.cfgPanel?.classList.toggle("hidden", !active || !cfgOpen);
    els.volPresets?.querySelectorAll(".voice-seg-btn").forEach((b) => {
      b.classList.toggle("active", Number(b.dataset.vol) === currentVolPct);
    });
    els.langModes?.querySelectorAll(".voice-seg-btn").forEach((b) => {
      b.classList.toggle("active", b.dataset.vlang === effLang);
    });
    els.micModes?.querySelectorAll(".voice-seg-btn").forEach((b) => {
      b.classList.toggle("active", b.dataset.micMode === micMode);
    });
    if (els.micModeDesc) {
      const descKey = micMode === "strict"
        ? "voiceModeStrictDesc"
        : (micMode === "ptt" ? "voiceModePttDesc" : "voiceModeNearDesc");
      els.micModeDesc.textContent = t(descKey);
    }
  }

  function syncButtons() {
    els.chatPane?.classList.toggle("voice-stage-open", active);
    els.btn?.classList.toggle("active", active);
    els.bar?.classList.toggle("hidden", !active);
    els.textDrawer?.classList.toggle("hidden", !active || !kbdOpen);
    els.kbdBtn?.classList.toggle("active", active && kbdOpen);
    renderCfgUi();
    if (els.muteBtn) {
      const isPtt = micMode === "ptt";
      els.muteBtn.classList.toggle("muted", !isPtt && muted);
      els.muteBtn.classList.toggle("ptt-ready", isPtt && !pttTalking);
      els.muteBtn.classList.toggle("ptt-talking", isPtt && pttTalking);
      els.muteBtn.title = isPtt
        ? t("voiceModePttDesc")
        : (muted ? t("voiceUnmuteTitle") : t("voiceMuteTitle"));
    }
    if (micMode === "ptt") {
      if (els.muteIcon) els.muteIcon.textContent = pttTalking ? "🟢" : "🎙️";
      if (els.muteLabel) els.muteLabel.textContent = t(pttTalking ? "voicePttTalkingBtn" : "voicePttHoldBtn");
    } else {
      if (els.muteIcon) els.muteIcon.textContent = muted ? "🔇" : "🎤";
      if (els.muteLabel) els.muteLabel.textContent = t(muted ? "voiceMicMutedBtn" : "voiceMicOnBtn");
    }
  }

  function stopCall() {
    active = false;
    muted = false;
    kbdOpen = false;
    cfgOpen = false;
    pttTalking = false;
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
    const lang = encodeURIComponent(effectiveVoiceLang());
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
        const [stKey, stMode] = idleStatePair();
        setStatus(stKey, stMode);
        if (turns.length === 0 && els.sub) {
          els.sub.textContent = t(micMode === "ptt" ? "voicePttReadyHint" : "voiceReadyHint");
        }
      } else if (msg.type === "interrupted") {
        flushPlayback();
        closeOpenTurns();
        const [stKey, stMode] = idleStatePair();
        setStatus(stKey, stMode);
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
          const [stKey, stMode] = idleStatePair();
          setStatus(stKey, stMode);
        }
      } else if (msg.type === "error") {
        appendCaption("tool", `⚠️ ${msg.error || "Voice error"}`, true);
      }
    });

    sock.addEventListener("close", () => {
      if (vws !== sock || !active) return;
      vws = null;
      flushPlayback();
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
      ? navigator.mediaDevices.getUserMedia(micConstraints()).catch(() => null)
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
    cfgOpen = false;
    pttTalking = false;
    noiseFloor = 0.002;
    speechOpenUntilMs = 0;
    voiceOnsetStreak = 0;
    preRollFrames = [];
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

  function setPttTalking(next) {
    if (!active || micMode !== "ptt" || pttTalking === next) return;
    pttTalking = next;
    if (pttTalking && playingSources.size > 0) {
      flushPlayback();
      closeOpenTurns();
    }
    syncButtons();
    if (playingSources.size === 0) {
      const [stKey, stMode] = idleStatePair();
      setStatus(stKey, stMode);
    }
  }

  els.btn?.addEventListener("click", () => {
    if (active) stopCall();
    else startCall();
  });

  els.cfgBtn?.addEventListener("click", () => {
    if (!active) return;
    cfgOpen = !cfgOpen;
    renderCfgUi();
  });

  els.volPresets?.querySelectorAll(".voice-seg-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const v = Number(b.dataset.vol) || 100;
      currentVolPct = v;
      currentGain = v / 100;
      try { localStorage.setItem("tuner_voice_vol", `${v}`); } catch (_) {}
      if (outGain) outGain.gain.value = currentGain;
      renderCfgUi();
    });
  });

  els.langModes?.querySelectorAll(".voice-seg-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const next = b.dataset.vlang || "ko";
      const normalized = next === "en" || next === "auto" ? next : "ko";
      if (normalized === effectiveVoiceLang() && voiceLang === normalized) return;
      voiceLang = normalized;
      try { localStorage.setItem("tuner_voice_lang", voiceLang); } catch (_) {}
      renderCfgUi();
      if (active) {
        flushPlayback();
        closeOpenTurns();
        if (reconnectTimer) {
          clearTimeout(reconnectTimer);
          reconnectTimer = null;
        }
        reconnectAttempts = 0;
        if (vws) {
          const old = vws;
          vws = null;
          try { old.close(); } catch (_) {}
        }
        setStatus("voiceConnecting", "thinking");
        openVoiceSocket(null);
      }
    });
  });

  els.micModes?.querySelectorAll(".voice-seg-btn").forEach((b) => {
    b.addEventListener("click", () => {
      const next = b.dataset.micMode || "near";
      micMode = next === "strict" || next === "ptt" ? next : "near";
      pttTalking = false;
      speechOpenUntilMs = 0;
      voiceOnsetStreak = 0;
      try { localStorage.setItem("tuner_voice_mic_mode", micMode); } catch (_) {}
      syncButtons();
      if (turns.length === 0 && els.sub) {
        els.sub.textContent = t(micMode === "ptt" ? "voicePttReadyHint" : "voiceReadyHint");
      }
      if (playingSources.size === 0) {
        const [stKey, stMode] = idleStatePair();
        setStatus(stKey, stMode);
      }
    });
  });

  // Tap Audio Orb while assistant is speaking to instantly interrupt playback (manual 0ms Barge-In)!
  els.wave?.addEventListener("click", () => {
    if (active && playingSources.size > 0) {
      flushPlayback();
      closeOpenTurns();
      const [stKey, stMode] = idleStatePair();
      setStatus(stKey, stMode);
    }
  });

  els.muteBtn?.addEventListener("pointerdown", (e) => {
    if (!active || micMode !== "ptt") return;
    e.preventDefault();
    try { els.muteBtn.setPointerCapture(e.pointerId); } catch (_) {}
    pttDownAt = performance.now();
    if (pttTalking) {
      // Tapping again while toggled ON finishes talking
      pttDownAt = 0;
      setPttTalking(false);
    } else {
      setPttTalking(true);
    }
  });

  const endPttHold = (e) => {
    if (!active || micMode !== "ptt" || !pttDownAt) return;
    const heldMs = performance.now() - pttDownAt;
    pttDownAt = 0;
    try { els.muteBtn.releasePointerCapture(e.pointerId); } catch (_) {}
    // If held >= 260ms, release ends talk (walkie-talkie hold); if quick-tapped < 260ms, stay latched until next tap!
    if (heldMs >= 260) {
      setPttTalking(false);
    }
  };
  els.muteBtn?.addEventListener("pointerup", endPttHold);
  els.muteBtn?.addEventListener("pointercancel", endPttHold);

  els.muteBtn?.addEventListener("click", () => {
    if (!active || micMode === "ptt") return;
    muted = !muted;
    syncButtons();
    if (playingSources.size === 0) {
      const [stKey, stMode] = idleStatePair();
      setStatus(stKey, stMode);
    }
  });

  els.kbdBtn?.addEventListener("click", () => {
    if (!active) return;
    kbdOpen = !kbdOpen;
    if (kbdOpen) cfgOpen = false;
    syncButtons();
    if (kbdOpen) setTimeout(() => els.input?.focus(), 20);
  });

  els.sendTextBtn?.addEventListener("click", submitTextDrawer);

  els.hangupBtn?.addEventListener("click", stopCall);

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
