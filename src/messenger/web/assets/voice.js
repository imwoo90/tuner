import { t } from "/dashboard/assets/i18n.js";

const WORKLET_CODE = `
class Pcm16CaptureProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buf = new Int16Array(1024);
    this.off = 0;
  }
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (!ch || !ch.length) return true;
    const ratio = sampleRate / 16000;
    let i = 0;
    while (i < ch.length) {
      const idx = Math.min(ch.length - 1, Math.floor(i));
      const s = Math.max(-1, Math.min(1, ch[idx]));
      this.buf[this.off++] = s < 0 ? s * 0x8000 : s * 0x7fff;
      if (this.off >= this.buf.length) {
        const copy = this.buf.slice(0);
        this.port.postMessage(copy.buffer, [copy.buffer]);
        this.off = 0;
      }
      i += ratio;
    }
    return true;
  }
}
registerProcessor("pcm16-capture", Pcm16CaptureProcessor);
`;

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

export function createVoiceController({
  apiFetch,
  getCsrfToken,
  getLang,
  getActiveSessionKey,
  onFocusTopic,
  onNeedGeminiSetup
}) {
  const els = {
    btn: document.getElementById("btn-voice"),
    bar: document.getElementById("voice-bar"),
    wave: document.getElementById("voice-wave"),
    badge: document.getElementById("voice-status-badge"),
    sub: document.getElementById("voice-subtitle"),
    input: document.getElementById("voice-quick-input"),
    muteBtn: document.getElementById("btn-voice-mute"),
    hangupBtn: document.getElementById("btn-voice-hangup")
  };

  let vws = null;
  let active = false;
  let muted = false;
  let audioCtx = null;
  let mediaStream = null;
  let workletNode = null;
  let nextPlayTime = 0;
  let botTranscriptBuf = "";
  const playingSources = new Set();

  window.__voiceAudioChunksReceived = 0;

  function setStatus(stateKey, mode = "listening") {
    if (els.badge) els.badge.textContent = t(stateKey);
    if (els.wave) els.wave.dataset.mode = muted ? "muted" : mode;
  }

  function flushPlayback() {
    for (const src of playingSources) {
      try { src.stop(); } catch (_) {}
    }
    playingSources.clear();
    if (audioCtx) nextPlayTime = audioCtx.currentTime;
  }

  function ensureAudioCtx() {
    if (!audioCtx) {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      if (Ctx) audioCtx = new Ctx();
    }
    if (audioCtx && audioCtx.state === "suspended") {
      audioCtx.resume().catch(() => {});
    }
    return audioCtx;
  }

  function enqueuePcm24(b64) {
    window.__voiceAudioChunksReceived = (window.__voiceAudioChunksReceived || 0) + 1;
    const ctx = ensureAudioCtx();
    if (!ctx) return;
    let pcm;
    try {
      pcm = base64ToInt16(b64);
    } catch (_) {
      return;
    }
    if (!pcm.length) return;
    const buf = ctx.createBuffer(1, pcm.length, 24000);
    const ch = buf.getChannelData(0);
    for (let i = 0; i < pcm.length; i++) {
      ch[i] = pcm[i] / 32768;
    }
    const src = ctx.createBufferSource();
    src.buffer = buf;
    src.connect(ctx.destination);
    const startAt = Math.max(ctx.currentTime + 0.015, nextPlayTime);
    nextPlayTime = startAt + buf.duration;
    playingSources.add(src);
    setStatus("voiceSpeaking", "speaking");
    src.onended = () => {
      playingSources.delete(src);
      if (playingSources.size === 0 && active) {
        setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
      }
    };
    try { src.start(startAt); } catch (_) { playingSources.delete(src); }
  }

  async function startMicCapture() {
    if (!navigator.mediaDevices?.getUserMedia) return;
    try {
      mediaStream = await navigator.mediaDevices.getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
          channelCount: 1
        }
      });
      const ctx = ensureAudioCtx();
      if (!ctx || !ctx.audioWorklet) return;
      const blob = new Blob([WORKLET_CODE], { type: "application/javascript" });
      const modUrl = URL.createObjectURL(blob);
      try {
        await ctx.audioWorklet.addModule(modUrl);
      } finally {
        URL.revokeObjectURL(modUrl);
      }
      if (!active) return;
      const source = ctx.createMediaStreamSource(mediaStream);
      workletNode = new AudioWorkletNode(ctx, "pcm16-capture");
      workletNode.port.onmessage = (ev) => {
        if (!active || muted || !vws || vws.readyState !== WebSocket.OPEN) return;
        const b64 = bytesToBase64(new Uint8Array(ev.data));
        vws.send(JSON.stringify({ type: "audio", data: b64 }));
      };
      source.connect(workletNode);
    } catch (_) {
      // Headless browser or denied mic permission: voice session remains active for spoken playback & quick input.
    }
  }

  function stopHardware() {
    flushPlayback();
    if (workletNode) {
      try { workletNode.disconnect(); } catch (_) {}
      workletNode = null;
    }
    if (mediaStream) {
      mediaStream.getTracks().forEach((tr) => tr.stop());
      mediaStream = null;
    }
  }

  function syncButtons() {
    els.btn?.classList.toggle("active", active);
    els.bar?.classList.toggle("hidden", !active);
    if (els.muteBtn) {
      els.muteBtn.textContent = muted ? "🔇" : "🎤";
      els.muteBtn.title = muted ? t("voiceUnmuteTitle") : t("voiceMuteTitle");
      els.muteBtn.classList.toggle("muted", muted);
    }
  }

  function stopCall() {
    active = false;
    muted = false;
    botTranscriptBuf = "";
    if (vws) {
      try { vws.close(); } catch (_) {}
      vws = null;
    }
    stopHardware();
    syncButtons();
  }

  async function startCall() {
    const stRes = await apiFetch("/dashboard/api/voice/status");
    if (!stRes.ok) return;
    const st = await stRes.json();
    if (!st.ready) {
      onNeedGeminiSetup?.();
      return;
    }
    ensureAudioCtx();
    active = true;
    muted = false;
    botTranscriptBuf = "";
    window.__voiceAudioChunksReceived = 0;
    syncButtons();
    setStatus("voiceConnecting", "thinking");
    if (els.sub) els.sub.textContent = t("voiceConnectingHint");

    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    const sk = encodeURIComponent(getActiveSessionKey() || "");
    const lang = encodeURIComponent(getLang() || "ko");
    const csrf = encodeURIComponent(getCsrfToken() || "");
    vws = new WebSocket(`${proto}//${window.location.host}/dashboard/api/voice/ws?csrf=${csrf}&lang=${lang}&topic=${sk}`);

    vws.addEventListener("open", () => {
      startMicCapture();
    });

    vws.addEventListener("message", (ev) => {
      let msg;
      try { msg = JSON.parse(ev.data); } catch (_) { return; }
      if (msg.type === "ready") {
        setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
        if (els.sub) els.sub.textContent = t("voiceReadyHint");
      } else if (msg.type === "interrupted") {
        flushPlayback();
        botTranscriptBuf = "";
        setStatus("voiceListening", "listening");
      } else if (msg.type === "audio" && msg.data) {
        enqueuePcm24(msg.data);
      } else if (msg.type === "user_transcript" && msg.text) {
        botTranscriptBuf = "";
        if (els.sub) els.sub.textContent = `🗣️ ${msg.text}`;
      } else if (msg.type === "bot_transcript" && msg.text) {
        botTranscriptBuf += msg.text;
        if (els.sub) els.sub.textContent = botTranscriptBuf.trim();
      } else if (msg.type === "tool_call" && msg.name) {
        setStatus("voiceThinking", "thinking");
        if (els.sub) els.sub.textContent = `⚡ ${msg.name}`;
      } else if (msg.type === "focus_topic" && msg.session_key) {
        onFocusTopic?.(msg.session_key);
      } else if (msg.type === "turn_complete") {
        botTranscriptBuf = "";
        if (playingSources.size === 0) {
          setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
        }
      } else if (msg.type === "error") {
        if (els.sub) els.sub.textContent = `⚠️ ${msg.error || "Voice error"}`;
      }
    });

    vws.addEventListener("close", () => {
      if (active) stopCall();
    });
  }

  function sendText(text) {
    const clean = (text || "").trim();
    if (!clean || !vws || vws.readyState !== WebSocket.OPEN) return false;
    botTranscriptBuf = "";
    setStatus("voiceThinking", "thinking");
    if (els.sub) els.sub.textContent = `🗣️ ${clean}`;
    vws.send(JSON.stringify({ type: "text", text: clean }));
    return true;
  }

  els.btn?.addEventListener("click", () => {
    if (active) stopCall();
    else startCall();
  });

  els.muteBtn?.addEventListener("click", () => {
    if (!active) return;
    muted = !muted;
    syncButtons();
    if (playingSources.size === 0) {
      setStatus(muted ? "voiceMuted" : "voiceListening", muted ? "muted" : "listening");
    }
  });

  els.hangupBtn?.addEventListener("click", stopCall);

  els.input?.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (sendText(els.input.value)) els.input.value = "";
    }
  });

  return {
    isActive: () => active,
    sendText,
    notifyActiveTopic: (sk) => {
      if (active && vws && vws.readyState === WebSocket.OPEN) {
        vws.send(JSON.stringify({ type: "active_topic", session_key: sk || "" }));
      }
    },
    refreshI18n: () => {
      if (active) syncButtons();
    }
  };
}
