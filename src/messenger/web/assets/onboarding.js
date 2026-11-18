import { escapeHtml } from "/dashboard/assets/markdown.js";
import { t } from "/dashboard/assets/i18n.js";

export function createOnboardingRenderer({ apiFetch, onOpenBrowserToken, onProviderReady, onCloseManager }) {
  let pollTimer = null;

  function clearPoll() {
    if (pollTimer) {
      clearTimeout(pollTimer);
      pollTimer = null;
    }
  }

  function renderSingleProviderCard(p, showUseBtn = false) {
    const ready = Boolean(p.installed && p.authenticated);
    const installing = Boolean(p.install_job?.running);
    const badgeHtml = ready
      ? `<span class="pill pill-ok">${escapeHtml(t("provReadyBadge"))}</span>`
      : p.installed
        ? `<span class="pill pill-warn">${escapeHtml(t("provNeedAuthBadge"))}</span>`
        : `<span class="pill">${escapeHtml(t("provMissingBadge"))}</span>`;

    const verStr = p.version ? `${p.version} · ` : "";
    let actionsHtml = "";

    if (!p.installed || installing) {
      actionsHtml += `<div class="card-row">
        <button class="btn btn-primary btn-sm prov-install-btn" data-prov="${escapeHtml(p.id)}" type="button" ${installing ? "disabled" : ""}>
          ${installing ? `<span class="loading-spinner spinner-xs"></span> ${escapeHtml(t("provInstallingBtn"))}` : escapeHtml(t("provInstallBtn"))}
        </button>
      </div>`;
    } else if (!p.authenticated) {
      actionsHtml += `<div class="card-row">
        <button class="btn btn-primary btn-sm prov-weblogin-btn" data-prov="${escapeHtml(p.id)}" type="button">${escapeHtml(t("provWebInWebLoginBtn"))}</button>
        <button class="btn btn-sm prov-extlogin-btn" data-prov="${escapeHtml(p.id)}" type="button">${escapeHtml(t("provExtLoginBtn"))}</button>
      </div>`;
    } else {
      const useHtml = showUseBtn
        ? `<button class="btn btn-primary btn-sm prov-use-engine-btn" data-prov="${escapeHtml(p.id)}" type="button">${escapeHtml(t("useThisEngineBtn"))}</button>`
        : "";
      actionsHtml += `<div class="card-row">
        <div class="card-row">${useHtml}<button class="btn btn-sm prov-weblogin-btn" data-prov="${escapeHtml(p.id)}" type="button">${escapeHtml(t("provReloginBtn"))}</button></div>
      </div>`;
    }

    if (p.install_job?.log_tail && (installing || !p.installed)) {
      actionsHtml += `<pre class="code-pre install-log-pre">${escapeHtml(p.install_job.log_tail)}</pre>`;
    }

    if (p.oauth_login) {
      const hintKey = p.id === "antigravity" ? "provAgyCodeHint" : "provAutoCallbackHint";
      actionsHtml += `<div class="onboard-oauth-box" data-oauth-box="${escapeHtml(p.id)}">
        <div class="muted">${escapeHtml(t(hintKey))}</div>
        <div class="muted mono" style="font-size:11px;"><a href="${escapeHtml(p.oauth_login.oauth_url)}" target="_blank" rel="noopener">${escapeHtml(p.oauth_login.oauth_url.slice(0, 72))}… ↗</a></div>
        <div class="ask-writein-row">
          <input class="oauth-code-input" data-prov="${escapeHtml(p.id)}" type="text" placeholder="${escapeHtml(t("provCodePlaceholder"))}">
          <button class="btn btn-primary btn-sm prov-submit-code-btn" data-prov="${escapeHtml(p.id)}" type="button">${escapeHtml(t("provCodeSubmitBtn"))}</button>
          <button class="btn btn-sm prov-cancel-login-btn" data-prov="${escapeHtml(p.id)}" type="button">${escapeHtml(t("cancel"))}</button>
        </div>
      </div>`;
    }

    if (p.installed && (p.id === "claude" || p.id === "codex")) {
      actionsHtml += `<details class="cot-block">
        <summary>${escapeHtml(t("provApiKeySummary"))}</summary>
        <div class="ask-writein-row" style="margin-top:6px;">
          <input class="prov-apikey-input" data-prov="${escapeHtml(p.id)}" type="password" placeholder="${escapeHtml(t("provApiKeyPh"))}">
          <button class="btn btn-sm prov-save-apikey-btn" data-prov="${escapeHtml(p.id)}" type="button">${escapeHtml(t("provApiKeySaveBtn"))}</button>
        </div>
      </details>`;
    }

    return `<div class="card onboard-prov-card" data-prov-card="${escapeHtml(p.id)}">
      <div class="card-row"><strong>${escapeHtml(p.name)}</strong>${badgeHtml}</div>
      <div class="muted">${escapeHtml(verStr + (p.auth_detail || ""))}</div>
      ${actionsHtml}
    </div>`;
  }

  async function renderInto(rootEl, { isWizard = false, isManager = false } = {}) {
    clearPoll();
    if (!rootEl) return;
    const isCenter = isWizard || isManager;
    const [res, tgRes] = await Promise.all([
      apiFetch("/dashboard/api/providers/status"),
      !isWizard ? apiFetch("/dashboard/api/telegram/status").catch(() => null) : Promise.resolve(null)
    ]);
    if (!res.ok || !rootEl.isConnected) return;
    const data = await res.json();
    const tgData = tgRes && tgRes.ok ? await tgRes.json().catch(() => null) : null;
    if (!rootEl.isConnected) return;

    const providers = data.providers || [];
    const anyReady = Boolean(data.any_ready);
    const firstReady = providers.find((p) => p.installed && p.authenticated)?.id || "";
    const wrapId = isWizard ? "onboarding-card" : isManager ? "engine-manager-card" : "engines-panel";

    let html = `<div id="${wrapId}" class="${isCenter ? "onboarding-wrap" : "engines-wrap"}">`;
    if (isWizard) {
      html += `<div class="onboard-header">
        <h2>🚀 ${escapeHtml(t("onboardHeroTitle"))}</h2>
        <p>${escapeHtml(t("onboardHeroSub"))}</p>
      </div>`;
      if (anyReady) {
        html += `<div class="card onboard-ready-banner">
          <div class="card-row">
            <strong>${escapeHtml(t("onboardReadyBanner"))}</strong>
            <button id="btn-finish-onboarding" class="btn btn-primary btn-sm" data-ready-prov="${escapeHtml(firstReady)}" type="button">${escapeHtml(t("onboardStartChatBtn"))}</button>
          </div>
        </div>`;
      }
    } else if (isManager) {
      html += `<div class="card-row" style="margin-bottom:4px;">
        <div>
          <h2 style="margin:0;font-size:18px;font-weight:650;">⚙ ${escapeHtml(t("engineMgrTitle"))}</h2>
          <div class="muted">${escapeHtml(t("engineMgrSub"))}</div>
        </div>
        <button id="btn-close-engine-manager" class="btn btn-sm" type="button">${escapeHtml(t("backToChatBtn"))}</button>
      </div>`;
    }

    for (const p of providers) {
      html += renderSingleProviderCard(p, isManager);
    }
    if (!isWizard && tgData) {
      const tgOk = Boolean(tgData.connected);
      const tgBadge = tgOk
        ? `<span class="pill pill-ok">✓ ${escapeHtml(tgData.bot_username ? `@${tgData.bot_username}` : t("provReadyBadge"))}</span>`
        : `<span class="pill">${escapeHtml(t("tgNotConnectedBadge"))}</span>`;
      const tgAction = tgOk
        ? `<div class="card-row"><button id="btn-tg-disconnect" class="btn btn-danger btn-sm" type="button">${escapeHtml(t("tgDisconnectBtn"))}</button></div>`
        : `<div class="muted">${escapeHtml(t("tgDisconnectedHint"))}</div>`;
      html += `<div class="card onboard-prov-card">
        <div class="card-row"><strong>✈️ ${escapeHtml(t("tgEngineCardTitle"))}</strong>${tgBadge}</div>
        ${tgAction}
      </div>`;
    }
    html += `<div id="onboard-err-msg" class="alert alert-danger hidden"></div></div>`;
    rootEl.innerHTML = html;

    const showErr = (msg) => {
      const el = rootEl.querySelector("#onboard-err-msg");
      if (el) {
        el.textContent = msg;
        el.classList.remove("hidden");
      }
    };

    rootEl.querySelector("#btn-finish-onboarding")?.addEventListener("click", (e) => {
      const readyId = e.currentTarget.dataset.readyProv || "antigravity";
      onProviderReady?.(readyId);
    });

    rootEl.querySelector("#btn-close-engine-manager")?.addEventListener("click", () => {
      onCloseManager?.();
    });

    rootEl.querySelectorAll(".prov-use-engine-btn").forEach((btn) => {
      btn.addEventListener("click", () => {
        onProviderReady?.(btn.dataset.prov || "antigravity");
      });
    });

    rootEl.querySelector("#btn-tg-disconnect")?.addEventListener("click", async (e) => {
      e.currentTarget.disabled = true;
      await apiFetch("/dashboard/api/telegram/disconnect", { method: "POST" });
      renderInto(rootEl, { isWizard, isManager });
    });

    rootEl.querySelectorAll(".prov-install-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        btn.disabled = true;
        const r = await apiFetch("/dashboard/api/providers/install", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ provider: btn.dataset.prov })
        });
        if (!r.ok) {
          const d = await r.json().catch(() => ({}));
          showErr(d.error || `HTTP ${r.status}`);
        }
        renderInto(rootEl, { isWizard, isManager });
      });
    });

    const startLogin = async (prov, openWebInWeb, triggerBtn) => {
      triggerBtn.disabled = true;
      const r = await apiFetch("/dashboard/api/providers/login/start", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ provider: prov, open_web_in_web: openWebInWeb })
      });
      triggerBtn.disabled = false;
      if (!r.ok) {
        const d = await r.json().catch(() => ({}));
        showErr(d.error || `HTTP ${r.status}`);
        return;
      }
      const d = await r.json();
      if (openWebInWeb && d.hitl_session?.token) {
        onOpenBrowserToken?.(d.hitl_session.token);
        if (!isCenter) return;
      } else if (!openWebInWeb && d.login?.oauth_url) {
        window.open(d.login.oauth_url, "_blank", "noopener");
      }
      renderInto(rootEl, { isWizard, isManager });
    };

    rootEl.querySelectorAll(".prov-weblogin-btn").forEach((btn) => {
      btn.addEventListener("click", () => startLogin(btn.dataset.prov, true, btn));
    });
    rootEl.querySelectorAll(".prov-extlogin-btn").forEach((btn) => {
      btn.addEventListener("click", () => startLogin(btn.dataset.prov, false, btn));
    });

    rootEl.querySelectorAll(".prov-submit-code-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const prov = btn.dataset.prov;
        const inp = rootEl.querySelector(`.oauth-code-input[data-prov="${prov}"]`);
        const code = (inp?.value || "").trim();
        if (!code) return;
        btn.disabled = true;
        const r = await apiFetch("/dashboard/api/providers/login/submit", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ provider: prov, code })
        });
        btn.disabled = false;
        if (!r.ok) {
          const d = await r.json().catch(() => ({}));
          showErr(d.error || `HTTP ${r.status}`);
          return;
        }
        renderInto(rootEl, { isWizard, isManager });
      });
    });

    rootEl.querySelectorAll(".prov-cancel-login-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        await apiFetch("/dashboard/api/providers/login/cancel", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ provider: btn.dataset.prov })
        });
        renderInto(rootEl, { isWizard, isManager });
      });
    });

    rootEl.querySelectorAll(".prov-save-apikey-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const prov = btn.dataset.prov;
        const inp = rootEl.querySelector(`.prov-apikey-input[data-prov="${prov}"]`);
        const apiKey = (inp?.value || "").trim();
        if (!apiKey) return;
        btn.disabled = true;
        const r = await apiFetch("/dashboard/api/providers/apikey", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ provider: prov, api_key: apiKey })
        });
        btn.disabled = false;
        if (!r.ok) {
          const d = await r.json().catch(() => ({}));
          showErr(d.error || `HTTP ${r.status}`);
          return;
        }
        renderInto(rootEl, { isWizard, isManager });
      });
    });

    if (providers.some((p) => p.install_job?.running)) {
      pollTimer = setTimeout(() => renderInto(rootEl, { isWizard, isManager }), 900);
    }
  }

  return { renderInto, clearPoll };
}
